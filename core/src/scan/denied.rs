//! Complete denied-path reports with disk-backed storage for low-memory scans.

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use serde::ser::{SerializeSeq, Serializer};

use crate::error::{Error, Result};
use crate::snapshot::fsutil;

#[derive(Debug, Clone)]
pub struct DeniedPaths {
    examples: Vec<String>,
    count: usize,
    spool: Option<Arc<Mutex<File>>>,
}

impl From<Vec<String>> for DeniedPaths {
    fn from(paths: Vec<String>) -> Self {
        Self {
            count: paths.len(),
            examples: paths,
            spool: None,
        }
    }
}

impl DeniedPaths {
    pub(crate) fn on_disk(dir: &Path) -> Result<Self> {
        let path = dir.join("snapshot.denied.tmp");
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|e| Error::io("create denied-path spool", e))?;
        // The open descriptor keeps the spool alive across publication and concurrent
        // scans; unlink now so process termination cannot leak it.
        std::fs::remove_file(&path).map_err(|e| Error::io("unlink denied-path spool", e))?;
        fsutil::secure_file(&file).map_err(|e| Error::io("secure denied-path spool", e))?;
        Ok(Self {
            examples: Vec::new(),
            count: 0,
            spool: Some(Arc::new(Mutex::new(file))),
        })
    }

    pub(crate) fn push(&mut self, path: &[u8]) -> Result<()> {
        let text = String::from_utf8_lossy(path);
        let mut file = self
            .spool
            .as_ref()
            .expect("disk-backed report")
            .lock()
            .unwrap();
        file.write_all(&(text.len() as u64).to_le_bytes())
            .and_then(|()| file.write_all(text.as_bytes()))
            .map_err(|e| Error::io("write denied-path spool", e))?;
        if self.examples.len() < 5 {
            self.examples.push(text.into_owned());
        }
        self.count += 1;
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.count
    }
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    pub fn warning(&self) -> Option<String> {
        if self.is_empty() {
            return None;
        }
        let shown: Vec<&str> = self.examples.iter().take(5).map(String::as_str).collect();
        let more = if self.count > 5 {
            format!(" and {} more", self.count - 5)
        } else {
            String::new()
        };
        Some(format!(
            "{} director{} could not be read, sizes under them are unknown: {}{more}",
            self.count,
            if self.count == 1 { "y" } else { "ies" },
            shown.join(", ")
        ))
    }
}

impl Serialize for DeniedPaths {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        let Some(spool) = &self.spool else {
            return self.examples.serialize(serializer);
        };
        let mut file = spool.lock().map_err(serde::ser::Error::custom)?;
        file.seek(SeekFrom::Start(0))
            .map_err(serde::ser::Error::custom)?;
        let mut seq = serializer.serialize_seq(Some(self.count))?;
        for _ in 0..self.count {
            let mut length = [0; 8];
            file.read_exact(&mut length)
                .map_err(serde::ser::Error::custom)?;
            let mut bytes = vec![0; u64::from_le_bytes(length) as usize];
            file.read_exact(&mut bytes)
                .map_err(serde::ser::Error::custom)?;
            let text = String::from_utf8(bytes).map_err(serde::ser::Error::custom)?;
            seq.serialize_element(&text)?;
        }
        seq.end()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn disk_report_streams_every_path_and_keeps_only_warning_examples() {
        let dir = tempfile::tempdir().unwrap();
        let mut report = DeniedPaths::on_disk(dir.path()).unwrap();
        for id in 0..1000 {
            report.push(format!("/denied/{id}").as_bytes()).unwrap();
        }
        report.push(b"/bad-\xff").unwrap();
        assert_eq!(report.len(), 1001);
        assert_eq!(report.examples.len(), 5);
        assert!(report.warning().unwrap().ends_with("and 996 more"));
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
        let clone = report.clone();
        let first = serde_json::to_value(&report).unwrap();
        assert_eq!(first.as_array().unwrap().len(), 1001);
        assert_eq!(first[999], "/denied/999");
        assert_eq!(first[1000], "/bad-\u{fffd}");
        assert_eq!(serde_json::to_value(&clone).unwrap(), first);
    }
}

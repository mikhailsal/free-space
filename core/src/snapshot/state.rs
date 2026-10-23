//! `state.json`: the mutable part of the snapshot directory (last error, last
//! success, generations, warnings). Written via tmp + rename under the lock.

use std::io::ErrorKind;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{SnapshotDir, fsutil};
use crate::platform::now_secs;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LastError {
    pub ts: i64,
    /// Step that failed: "scan", "refresh", "precheck", "write", "reserve", "publish".
    pub op: String,
    pub message: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    pub last_error: Option<LastError>,
    pub last_success_ts: Option<i64>,
    pub prev_generation: Option<u64>,
    pub current_generation: Option<u64>,
    /// Anomalies of the last successful scan (D13), e.g. unreadable directories.
    pub warnings: Vec<String>,
    pub last_scan_seconds: Option<f64>,
}

/// `Ok(None)` when the file does not exist, `Err(reason)` when it exists but
/// cannot be read or parsed (`status.state_error`).
pub fn read(dir: &SnapshotDir) -> Result<Option<State>, String> {
    match std::fs::read(dir.state()) {
        Ok(b) => serde_json::from_slice(&b)
            .map(Some)
            .map_err(|e| format!("state.json is corrupt: {e}")),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("state.json unreadable: {e}")),
    }
}

pub fn write(dir: &SnapshotDir, st: &State) -> std::io::Result<()> {
    let bytes = serde_json::to_vec_pretty(st).map_err(std::io::Error::other)?;
    fsutil::write_atomic(&dir.state(), &bytes)
}

/// Record a failure without touching the snapshots.
pub fn record_error(dir: &SnapshotDir, op: &str, message: &str) {
    let mut st = read(dir).ok().flatten().unwrap_or_default();
    st.last_error = Some(LastError {
        ts: now_secs(),
        op: op.to_string(),
        message: message.to_string(),
    });
    let _ = write(dir, &st);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_corrupt_and_roundtrip() {
        let d = tempfile::tempdir().unwrap();
        let dir = SnapshotDir::new(d.path());
        assert_eq!(read(&dir), Ok(None));
        std::fs::write(dir.state(), b"{not json").unwrap();
        assert!(read(&dir).unwrap_err().contains("corrupt"));
        let st = State {
            current_generation: Some(3),
            warnings: vec!["x".into()],
            ..Default::default()
        };
        write(&dir, &st).unwrap();
        assert_eq!(read(&dir).unwrap().unwrap(), st);
        record_error(&dir, "precheck", "no space");
        let st2 = read(&dir).unwrap().unwrap();
        assert_eq!(st2.current_generation, Some(3));
        assert_eq!(st2.last_error.unwrap().op, "precheck");
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(dir.state()).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o640);
    }
}

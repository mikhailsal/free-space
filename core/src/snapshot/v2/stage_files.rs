//! Recover abandoned staging files without touching another scanner's live file.
use crate::{Error, Result, snapshot::fsutil};
use std::{
    fs::{self, File, OpenOptions},
    os::{
        fd::AsRawFd,
        unix::{
            ffi::OsStrExt,
            fs::{MetadataExt, OpenOptionsExt},
        },
    },
    path::{Path, PathBuf},
};

pub(super) struct StageFile {
    file: File,
    path: PathBuf,
}
impl StageFile {
    pub(super) fn create(dir: &Path) -> Result<Self> {
        recover(dir)?;
        for _ in 0..16 {
            let path = dir.join(format!(
                "scan-{}.stage.tmp",
                crate::snapshot::writer::snapshot_id()
            ));
            let file = match OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .mode(fsutil::FILE_MODE)
                .custom_flags(libc::O_NOFOLLOW)
                .open(&path)
            {
                Ok(file) => file,
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(Error::storage_io("create v2 staging file", e)),
            };
            let stage = Self { file, path };
            // A cleaner can open the new name before we acquire its flock. If
            // it won, retry; SQLite has not opened or written this file yet.
            if !lock(&stage.file)? || !same_file(&stage.file, &stage.path)? {
                continue;
            }
            fsutil::secure_file(&stage.file).map_err(|e| Error::io("secure v2 staging file", e))?;
            return Ok(stage);
        }
        Err(Error::generic(
            "could not acquire a unique v2 staging file; retry scan",
        ))
    }
    pub(super) fn path(&self) -> &Path {
        &self.path
    }
}
impl Drop for StageFile {
    fn drop(&mut self) {
        // Preserve a replacement pathname; the open inode belongs to this guard.
        if same_file(&self.file, &self.path).unwrap_or(false) {
            let _ = fs::remove_file(&self.path);
        }
    }
}
fn matches(name: &[u8]) -> bool {
    name.len() == 31
        && name.starts_with(b"scan-")
        && name.ends_with(b".stage.tmp")
        && name[5..21]
            .iter()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b))
}
fn lock(file: &File) -> Result<bool> {
    let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if result == 0 {
        return Ok(true);
    }
    let error = std::io::Error::last_os_error();
    if error.kind() == std::io::ErrorKind::WouldBlock {
        return Ok(false);
    }
    Err(Error::io("lock v2 staging file", error))
}
fn same_file(file: &File, path: &Path) -> Result<bool> {
    let open = file
        .metadata()
        .map_err(|e| Error::io("inspect v2 staging file", e))?;
    let named = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(Error::io("inspect v2 staging name", e)),
    };
    Ok(open.is_file()
        && open.nlink() == 1
        && named.is_file()
        && open.dev() == named.dev()
        && open.ino() == named.ino())
}
fn recover(dir: &Path) -> Result<()> {
    for entry in fs::read_dir(dir).map_err(|e| Error::io("list v2 staging files", e))? {
        let entry = entry.map_err(|e| Error::io("read v2 staging name", e))?;
        if !matches(entry.file_name().as_bytes()) {
            continue;
        }
        let path = entry.path();
        let file = match OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(&path)
        {
            Ok(file) => file,
            Err(e)
                if matches!(
                    e.raw_os_error(),
                    Some(libc::ENOENT | libc::ELOOP | libc::EACCES)
                ) =>
            {
                continue;
            }
            Err(e) => return Err(Error::io("open stale v2 staging file", e)),
        };
        if same_file(&file, &path)? && lock(&file)? && same_file(&file, &path)? {
            match fs::remove_file(&path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(Error::io("remove stale v2 staging file", e)),
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;

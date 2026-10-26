//! Secure creation for the live store. SQLite inherits DB modes for WAL/SHM.
use crate::{Error, Result, snapshot::fsutil};
use rusqlite::Connection;
use std::{fs::OpenOptions, os::unix::fs::OpenOptionsExt, path::Path};

pub(super) fn open(path: &Path) -> Result<Connection> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fsutil::ensure_dir(parent).map_err(|e| Error::storage_io("create v2 directory", e))?;
    match OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .mode(fsutil::FILE_MODE)
        .open(path)
    {
        Ok(file) => fsutil::secure_file(&file).map_err(|e| Error::io("secure new v2 store", e))?,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(Error::storage_io("create v2 store", e)),
    }
    Ok(Connection::open(path)?)
}
/// Called only after validating the store identity, so foreign/v1 files retain
/// their modes and ownership. Secure existing sidecars before additional writes.
pub(super) fn secure(path: &Path) -> Result<()> {
    fsutil::secure_path(path).map_err(|e| Error::io("secure v2 store", e))?;
    for suffix in ["-wal", "-shm"] {
        let mut p = path.as_os_str().to_os_string();
        p.push(suffix);
        match fsutil::secure_path(Path::new(&p)) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(Error::io("secure v2 sidecar", e)),
        }
    }
    Ok(())
}

/// Keep initialized sidecars after the last connection closes: group-read-only
/// clients cannot recreate WAL/SHM files in a 0750 snapshot directory.
pub(super) fn persist_wal(conn: &Connection) -> Result<()> {
    let mut persist: std::ffi::c_int = 1;
    let rc = unsafe {
        rusqlite::ffi::sqlite3_file_control(
            conn.handle(),
            c"main".as_ptr(),
            rusqlite::ffi::SQLITE_FCNTL_PERSIST_WAL,
            (&mut persist as *mut std::ffi::c_int).cast(),
        )
    };
    if rc != rusqlite::ffi::SQLITE_OK {
        return Err(Error::generic(format!(
            "enable persistent v2 WAL sidecars: SQLite code {rc}"
        )));
    }
    Ok(())
}

/// SQLite can lose the filesystem errno or report an unreadable WAL as a
/// schema failure. Probe only after a failed open, leaving healthy readers'
/// I/O unchanged and retaining the original error unless access is denied.
pub(super) fn reader_error(path: &Path, error: Error) -> Error {
    let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    for suffix in ["", ".readers", "-wal", "-shm"] {
        let mut file = path.as_os_str().to_os_string();
        file.push(suffix);
        let file = Path::new(&file);
        if let Err(denied) = std::fs::File::open(file)
            && denied.kind() == std::io::ErrorKind::PermissionDenied
        {
            return crate::snapshot::reader::access_error(file, &denied).with_fix(
                "ask a human to run: sudo freespace --backend v2 install-timer (grants this user read access)",
            );
        }
    }
    error
}

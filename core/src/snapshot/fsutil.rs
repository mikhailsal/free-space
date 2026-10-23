//! File modes and ownership (DX file-mode table): snapshot files are
//! `0640 root:freespace`, set explicitly so the umask never matters.

use std::ffi::CString;
use std::fs::File;
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::Path;

pub const GROUP: &str = "freespace";
pub const FILE_MODE: u32 = 0o640;
pub const DIR_MODE: u32 = 0o750;

pub fn is_root() -> bool {
    unsafe { libc::geteuid() == 0 }
}

pub fn group_gid(name: &str) -> Option<u32> {
    let c = CString::new(name).ok()?;
    let g = unsafe { libc::getgrnam(c.as_ptr()) };
    if g.is_null() {
        None
    } else {
        Some(unsafe { (*g).gr_gid })
    }
}

/// `fchmod 0640` and, when running as root and the group exists, `fchown root:freespace`.
pub fn secure_file(f: &File) -> io::Result<()> {
    let fd = f.as_raw_fd();
    if unsafe { libc::fchmod(fd, FILE_MODE as libc::mode_t) } != 0 {
        return Err(io::Error::last_os_error());
    }
    if is_root()
        && let Some(gid) = group_gid(GROUP)
        && unsafe { libc::fchown(fd, 0, gid) } != 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

pub fn secure_path(p: &Path) -> io::Result<()> {
    let f = File::open(p)?;
    secure_file(&f)
}

/// Create the snapshot directory (0750, root:freespace when possible) if missing.
pub fn ensure_dir(p: &Path) -> io::Result<()> {
    if p.is_dir() {
        return Ok(());
    }
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(DIR_MODE)
        .create(p)?;
    std::fs::set_permissions(p, std::fs::Permissions::from_mode(DIR_MODE))?;
    if is_root()
        && let Some(gid) = group_gid(GROUP)
    {
        let c = CString::new(p.as_os_str().as_encoded_bytes())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "NUL in path"))?;
        if unsafe { libc::chown(c.as_ptr(), 0, gid) } != 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

pub fn fsync_dir(p: &Path) -> io::Result<()> {
    File::open(p)?.sync_all()
}

/// Write `bytes` to `path` atomically: tmp file in the same directory, fchmod,
/// fsync, rename.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    use std::io::Write;
    let tmp = path.with_extension(format!(
        "{}.tmp",
        path.extension().and_then(|e| e.to_str()).unwrap_or("")
    ));
    let mut f = File::create(&tmp)?;
    secure_file(&f)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    drop(f);
    std::fs::rename(&tmp, path)
}

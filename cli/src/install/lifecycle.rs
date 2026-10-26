//! The backend-specific installation manifest. V2 stays separate from v1.
use freespace_core::{
    Error, Result,
    config::{Backend, Config},
    platform::UserInfo,
    snapshot::{SnapshotDir, fsutil},
};
use std::path::{Path, PathBuf};

pub(super) fn v2_files(dir: &SnapshotDir) -> Vec<PathBuf> {
    [
        "generations.db",
        "generations.db-wal",
        "generations.db-shm",
        "generations.db-journal",
        "generations.db.readers",
        "state.json",
        "state.json.tmp",
        "progress.json",
        "progress.json.tmp",
        ".lock",
    ]
    .into_iter()
    .map(|name| dir.path.join(name))
    .collect()
}

pub(super) fn prepare_backend_dirs(config: &Config) -> Result<SnapshotDir> {
    let base = SnapshotDir::new(&config.snapshot_dir);
    if config.backend == Backend::V1 {
        super::prepare_snapshot_dir(&base)?;
        return Ok(base);
    }
    // The base grants traversal; do not re-secure v1 snapshot files in it.
    prepare_directory(base.as_path())?;
    let dir = SnapshotDir::new(base.path.join("v2"));
    if dir.path.is_symlink() {
        return Err(Error::generic("refusing a symlinked v2 snapshot directory"));
    }
    prepare_directory(dir.as_path())?;
    for file in v2_files(&dir) {
        if file.is_symlink() {
            return Err(Error::generic(format!(
                "refusing symlinked v2 file {}",
                file.display()
            )));
        }
        if file.exists() {
            fsutil::secure_path(&file)
                .map_err(|e| Error::io(format!("secure {}", file.display()), e))?;
        }
    }
    Ok(dir)
}

fn prepare_directory(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fsutil::ensure_dir(path).map_err(|e| Error::io(format!("create {}", path.display()), e))?;
    std::os::unix::fs::chown(path, Some(0), fsutil::group_gid(fsutil::GROUP))
        .map_err(|e| Error::io(format!("chown {}", path.display()), e))?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(fsutil::DIR_MODE))
        .map_err(|e| Error::io(format!("chmod {}", path.display()), e))
}

pub(super) fn grant_backend_access(
    config: &Config,
    dir: &SnapshotDir,
    user: &UserInfo,
) -> (&'static str, bool) {
    if config.backend == Backend::V1 {
        return super::grant_access(dir.as_path(), user);
    }
    let spec = format!("u:{}:rX", user.name);
    // Only traversal on the common parent; recursive/default ACLs belong to v2.
    if super::run_tool(
        "setfacl",
        &["-m", &spec, &config.snapshot_dir.display().to_string()],
    )
    .is_err()
    {
        return super::grant_group_access(user);
    }
    super::grant_access(dir.as_path(), user)
}

/// Hold the existing reader lease exclusively through deletion. Removing a WAL
/// with an in-flight reader would break its pinned generation transaction.
pub(super) fn reader_lease_for_cleanup(dir: &SnapshotDir) -> Result<Option<std::fs::File>> {
    use std::os::fd::AsRawFd;
    let path = dir.path.join("generations.db.readers");
    let file = match std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(Error::io("open v2 reader lease", error)),
    };
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err(Error::new(
            freespace_core::ErrorCode::Lock,
            "v2 readers are active; close queries before uninstalling",
        ));
    }
    Ok(Some(file))
}

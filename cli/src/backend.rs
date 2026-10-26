//! Explicit backend dispatch. V2 never opens or migrates the v1 database.
use freespace_core::{
    Error, ErrorCode, Result,
    config::{self, Backend, Loaded},
    queries::{self, params::*, types::*},
    snapshot::{SnapshotDir, reader::Snapshot, v2::Reader},
};
use std::sync::OnceLock;

static OVERRIDE: OnceLock<Backend> = OnceLock::new();
pub fn select(value: Option<&str>) {
    if let Some(value) = value {
        let _ = OVERRIDE.set(if value == "v2" {
            Backend::V2
        } else {
            Backend::V1
        });
    }
}
pub fn load() -> Result<Loaded> {
    let mut loaded = config::load()?;
    if let Some(value) = OVERRIDE.get() {
        loaded.config.backend = *value;
    }
    Ok(loaded)
}
pub fn dir(loaded: &Loaded) -> SnapshotDir {
    match loaded.config.backend {
        Backend::V1 => loaded.snapshot_dir(),
        Backend::V2 => SnapshotDir::new(loaded.config.snapshot_dir.join("v2")),
    }
}
pub fn database(dir: &SnapshotDir) -> std::path::PathBuf {
    dir.path.join("generations.db")
}
pub fn reader(dir: &SnapshotDir) -> Result<Reader> {
    let path = database(dir);
    if std::fs::metadata(&path).is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound) {
        return Err(Error::new(
            ErrorCode::NoSnapshot,
            format!("no snapshot in {}", dir.path.display()),
        )
        .with_fix("freespace --backend v2 scan"));
    }
    Reader::open(&path)
}
pub enum Session {
    V1(Snapshot),
    V2(Reader),
}
impl Session {
    pub fn open(loaded: &Loaded) -> Result<Self> {
        let dir = dir(loaded);
        match loaded.config.backend {
            Backend::V1 => Snapshot::open_current(&dir).map(Self::V1),
            Backend::V2 => reader(&dir).map(Self::V2),
        }
    }
    pub fn ls(&self, p: &LsParams) -> Result<LsResponse> {
        match self {
            Self::V1(s) => queries::ls(s, p),
            Self::V2(s) => queries::v2::ls(s, p),
        }
    }
    pub fn top(&self, p: &TopParams) -> Result<TopResponse> {
        match self {
            Self::V1(s) => queries::top(s, p),
            Self::V2(s) => queries::v2::top(s, p),
        }
    }
    pub fn find(&self, p: &FindParams) -> Result<FindResponse> {
        match self {
            Self::V1(s) => queries::find(s, p),
            Self::V2(s) => queries::v2::find(s, p),
        }
    }
}
pub fn diff(loaded: &Loaded, p: &DiffParams) -> Result<DiffResponse> {
    let dir = dir(loaded);
    match loaded.config.backend {
        Backend::V1 => queries::diff(&dir, p),
        Backend::V2 => queries::v2::diff(&reader(&dir)?, p),
    }
}
pub fn status(backend: Backend, dir: &SnapshotDir, allow_missing: bool) -> Result<StatusResponse> {
    if backend == Backend::V1 {
        return if allow_missing {
            queries::status::status_page(dir)
        } else {
            queries::status(dir)
        };
    }
    match reader(dir) {
        Ok(reader) => queries::v2::status(&reader, dir),
        Err(error)
            if error.code == ErrorCode::NoSnapshot
                && (allow_missing || freespace_core::snapshot::lock::probe(dir).is_some()) =>
        {
            queries::v2::status_page(None, dir)
        }
        Err(error) => Err(error),
    }
}

fn record<T>(dir: &SnapshotDir, op: &str, result: Result<T>) -> Result<T> {
    if let Err(error) = &result
        && !matches!(error.code, ErrorCode::Usage | ErrorCode::Lock)
    {
        freespace_core::snapshot::state::record_error(dir, op, &error.message);
    }
    result
}
fn success(dir: &SnapshotDir, reader: &Reader, seconds: f64, denied: &[String]) -> Result<()> {
    use freespace_core::snapshot::state;
    let mut state = state::read(dir).ok().flatten().unwrap_or_default();
    state.current_generation = Some(reader.generations.current as u64);
    state.prev_generation = reader.generations.prev.map(|g| g as u64);
    state.last_error = None;
    state.last_success_ts = Some(freespace_core::platform::now_secs());
    state.last_scan_seconds = Some(seconds);
    state.warnings = freespace_core::job::denied_warning(denied)
        .into_iter()
        .collect();
    state::write(dir, &state).map_err(|e| Error::io("write state.json", e))
}
fn scan_store(
    store: &mut freespace_core::snapshot::v2::Store,
    roots: &[std::path::PathBuf],
    one_file_system: bool,
    workers: usize,
) -> Result<freespace_core::snapshot::v2::WriteReport> {
    store.scan_roots(roots, one_file_system, workers)
}
pub fn scan(
    loaded: &Loaded,
    roots: &[std::path::PathBuf],
    one_file_system: bool,
    workers: usize,
    low_memory: bool,
) -> Result<freespace_core::job::ScanReport> {
    use freespace_core::snapshot::{fsutil, lock, v2::Store, writer::WriteStats};
    if low_memory || !loaded.config.keep_previous {
        return Err(Error::new(
            ErrorCode::NotSupported,
            "v2 does not yet support low_memory=true or keep_previous=false",
        ));
    }
    let dir = dir(loaded);
    fsutil::ensure_dir(dir.as_path())
        .map_err(|e| lock::open_error("scan", "", dir.as_path(), e))?;
    let label = roots
        .iter()
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>()
        .join(" ");
    let _lock = lock::acquire_with_path_arg(&dir, "scan", &label, None)?;
    record(
        &dir,
        "scan",
        (|| {
            let started = std::time::Instant::now();
            let mut store = Store::open(&database(&dir))?;
            let write = scan_store(&mut store, roots, one_file_system, workers)?;
            let reader = reader(&dir)?;
            let status = queries::v2::status(&reader, &dir)?;
            let bytes = snapshot_bytes(&dir)?;
            let total_seconds = started.elapsed().as_secs_f64();
            success(&dir, &reader, total_seconds, &status.unknown_subtrees)?;
            Ok(freespace_core::job::ScanReport {
                generation: reader.generations.current as u64,
                prev_generation: reader.generations.prev.map(|g| g as u64),
                entries: status.entries.unwrap_or_default(),
                walk_seconds: write.walk.as_secs_f64(),
                finalize_seconds: 0.0,
                write: WriteStats {
                    bytes,
                    insert_seconds: write.transaction.as_secs_f64(),
                    ..Default::default()
                },
                publish_seconds: 0.0,
                total_seconds,
                snapshot_bytes: bytes,
                denied: status.unknown_subtrees.into(),
                errors: reader
                    .roots(false)?
                    .iter()
                    .map(|r| {
                        reader
                            .get_id(r.entry_id, false)
                            .map(|e| e.map_or(0, |e| e.errors as u64))
                    })
                    .collect::<Result<Vec<_>>>()?
                    .into_iter()
                    .sum(),
            })
        })(),
    )
}
pub fn refresh(
    loaded: &Loaded,
    path: &std::path::Path,
    workers: usize,
) -> Result<freespace_core::refresh::RefreshReport> {
    use freespace_core::snapshot::{lock, v2::Store, writer::WriteStats};
    use std::os::unix::ffi::OsStrExt;
    let normalized = queries::common::normalize(path.as_os_str().as_bytes())?;
    let path = std::path::Path::new(std::ffi::OsStr::from_bytes(&normalized));
    let dir = dir(loaded);
    // Check existence before opening a writer, which would otherwise create an empty store.
    drop(reader(&dir)?);
    let _lock = lock::acquire_with_path_arg(&dir, "refresh", &path.display().to_string(), None)?;
    record(
        &dir,
        "refresh",
        (|| {
            let started = std::time::Instant::now();
            let mut store = Store::open(&database(&dir))?;
            let write = store.refresh(path, workers)?;
            let reader = reader(&dir)?;
            let status = queries::v2::status(&reader, &dir)?;
            let bytes = snapshot_bytes(&dir)?;
            let total_seconds = started.elapsed().as_secs_f64();
            success(&dir, &reader, total_seconds, &status.unknown_subtrees)?;
            Ok(freespace_core::refresh::RefreshReport {
                generation: reader.generations.current as u64,
                path: path.display().to_string(),
                entries: status.entries.unwrap_or_default(),
                subtree_entries: write.new_subtree_entries as u64,
                removed: write.new_subtree_entries == 0,
                load_seconds: 0.0,
                walk_seconds: write.walk.as_secs_f64(),
                finalize_seconds: 0.0,
                write: WriteStats {
                    bytes,
                    insert_seconds: write.transaction.as_secs_f64(),
                    ..Default::default()
                },
                publish_seconds: 0.0,
                total_seconds,
                snapshot_bytes: bytes,
                denied: status.unknown_subtrees,
            })
        })(),
    )
}

/// Same bounded descent as v1's status hint, using the request's existing pin.
pub fn top_growth(reader: &Reader, start: &[u8]) -> Result<Option<(String, i64)>> {
    use freespace_core::scan::Kind;
    let (Some(current), Some(previous)) = (reader.get(start, false)?, reader.get(start, true)?)
    else {
        return Ok(None);
    };
    let mut path = start.to_vec();
    let mut delta = current.allocated - previous.allocated;
    if delta <= 0 {
        return Ok(None);
    }
    for _ in 0..12 {
        let previous = reader.children(&path, true)?;
        let best = reader
            .children(&path, false)?
            .into_iter()
            .filter(|e| e.kind == Kind::Dir)
            .map(|entry| {
                let before = previous.iter().find(|p| p.path == entry.path);
                (
                    entry.allocated - before.map_or(0, |p| p.allocated),
                    entry,
                    before.is_some(),
                )
            })
            .max_by_key(|(delta, _, _)| *delta);
        match best {
            Some((growth, entry, existed)) if growth > 0 && growth.saturating_mul(2) >= delta => {
                path = entry.path;
                delta = growth;
                if !existed {
                    break;
                }
            }
            _ => break,
        }
    }
    Ok(Some((String::from_utf8_lossy(&path).into_owned(), delta)))
}

/// V2 combines finalization with walking and indexes/publication with its SQL
/// transaction. The existing numeric report assigns these combined durations to
/// walk_seconds and insert_seconds; phases without a separate step stay zero.
pub fn timing_note() {
    eprintln!(
        "v2 timings: walk includes finalization; insert includes indexing and publication (no separate phase timings)"
    );
}

/// Persistent storage is the database plus committed/uncheckpointed WAL pages.
/// SHM is a transient coordination index and is excluded, as are reader leases.
fn snapshot_bytes(dir: &SnapshotDir) -> Result<u64> {
    let path = database(dir);
    let database = std::fs::metadata(&path)
        .map_err(|e| Error::io("stat v2 store", e))?
        .len();
    let mut wal = path.into_os_string();
    wal.push("-wal");
    let wal = match std::fs::metadata(std::path::Path::new(&wal)) {
        Ok(meta) => meta.len(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
        Err(error) => return Err(Error::io("stat v2 WAL", error)),
    };
    database
        .checked_add(wal)
        .ok_or_else(|| Error::generic("v2 snapshot size overflow"))
}

#[cfg(test)]
mod tests {
    #[test]
    fn snapshot_size_includes_the_uncheckpointed_wal() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("root");
        std::fs::create_dir(&root).unwrap();
        let dir = freespace_core::snapshot::SnapshotDir::new(tmp.path());
        let path = super::database(&dir);
        let mut store = freespace_core::snapshot::v2::Store::open(&path).unwrap();
        store.scan(&root, 1).unwrap();
        let main = std::fs::metadata(&path).unwrap().len();
        let wal = std::fs::metadata(path.with_file_name("generations.db-wal"))
            .unwrap()
            .len();
        assert!(wal > 0);
        assert_eq!(super::snapshot_bytes(&dir).unwrap(), main + wal);
    }
}

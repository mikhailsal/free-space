//! `scan` end to end: lock → cleanup → walk → finalize → precheck → write tmp →
//! rotate prev → publish → state.json. `refresh` reuses the same tail.

use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::time::Instant;

use crate::error::{Error, ErrorCode, Result};
use crate::fault;
use crate::platform;
use crate::scan::{self, Finalized, NO_PARENT, Tree};
use crate::snapshot::progress::{self, ProgressCallback, ProgressWriter};
use crate::snapshot::reader::Snapshot;
use crate::snapshot::writer::{self, Reserve, WriteMeta, WriteStats};
use crate::snapshot::{SnapshotDir, fsutil, lock, publish, state};

fn scan_root_args(roots: &[PathBuf], one_file_system: bool, workers: usize) -> Option<String> {
    if roots.is_empty() {
        return None;
    }
    let mut args = Vec::with_capacity(roots.len());
    for root in roots {
        let bytes = root.as_os_str().as_bytes();
        let path = std::str::from_utf8(bytes).ok()?;
        if path.chars().any(char::is_control) {
            return None;
        }
        args.push(format!("'{}'", path.replace('\'', "'\\''")));
    }
    if !one_file_system {
        args.push("--cross-mounts".to_string());
    }
    args.push(format!("--workers {workers}"));
    Some(args.join(" "))
}

#[cfg(test)]
mod tests {
    use super::scan_root_args;
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    use std::path::PathBuf;

    #[test]
    fn scan_root_args_quote_each_utf8_root_and_refuse_unrepresentable_paths() {
        let roots = vec![PathBuf::from("/mnt/a b"), PathBuf::from("/mnt/a'b")];
        assert_eq!(
            scan_root_args(&roots, true, 4).as_deref(),
            Some("'/mnt/a b' '/mnt/a'\\''b' --workers 4")
        );
        assert_eq!(
            scan_root_args(&roots, false, 7).as_deref(),
            Some("'/mnt/a b' '/mnt/a'\\''b' --cross-mounts --workers 7")
        );
        assert_eq!(
            scan_root_args(
                &[PathBuf::from(OsString::from_vec(b"/mnt/\xff".to_vec()))],
                true,
                2
            ),
            None
        );
        assert_eq!(scan_root_args(&[PathBuf::from("/mnt/a\tb")], true, 2), None);
    }
}

pub struct ScanRequest {
    pub roots: Vec<PathBuf>,
    pub one_file_system: bool,
    pub workers: usize,
    pub dir: SnapshotDir,
    pub keep_previous: bool,
    pub on_progress: Option<ProgressCallback>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScanReport {
    pub generation: u64,
    pub prev_generation: Option<u64>,
    pub entries: u64,
    pub walk_seconds: f64,
    pub finalize_seconds: f64,
    pub write: WriteStats,
    pub publish_seconds: f64,
    pub total_seconds: f64,
    pub snapshot_bytes: u64,
    /// Paths of directories that could not be read (D13).
    pub denied: Vec<String>,
    pub errors: u64,
}

impl Tree {
    /// Full path of a raw node (parent chain walk).
    pub fn path_of(&self, raw: u32) -> Vec<u8> {
        let mut parts = Vec::new();
        let mut cur = raw;
        loop {
            let n = &self.nodes[cur as usize];
            parts.push(self.name(n));
            if n.parent == NO_PARENT {
                break;
            }
            cur = n.parent;
        }
        let mut p = parts.pop().unwrap_or_default().to_vec();
        while let Some(c) = parts.pop() {
            scan::tree::push_component(&mut p, c);
        }
        p
    }
}

pub fn denied_paths(tree: &Tree, fin: &Finalized) -> Vec<String> {
    fin.denied
        .iter()
        .map(|&id| String::from_utf8_lossy(&tree.path_of(fin.order[id as usize])).into_owned())
        .collect()
}

pub fn denied_warning(denied: &[String]) -> Option<String> {
    if denied.is_empty() {
        return None;
    }
    let shown: Vec<&str> = denied.iter().take(5).map(String::as_str).collect();
    let more = if denied.len() > 5 {
        format!(" and {} more", denied.len() - 5)
    } else {
        String::new()
    };
    Some(format!(
        "{} director{} could not be read, sizes under them are unknown: {}{more}",
        denied.len(),
        if denied.len() == 1 { "y" } else { "ies" },
        shown.join(", ")
    ))
}

/// Generation numbers for the next publish: (next, current).
pub fn next_generation(dir: &SnapshotDir) -> (u64, Option<u64>) {
    let cur = Snapshot::open(&dir.current())
        .ok()
        .map(|s| s.meta.generation);
    let st = state::read(dir)
        .ok()
        .flatten()
        .and_then(|s| s.current_generation);
    let base = cur.unwrap_or(0).max(st.unwrap_or(0));
    (base + 1, cur)
}

/// How a successful scan publishes its previous generation. Only snapshots
/// this binary can open are eligible; unreadable current files never replace prev.
enum Previous {
    Rotate(u64),
    Retain(u64),
    Drop,
}

impl Previous {
    fn for_scan(dir: &SnapshotDir, keep: bool, current: Option<u64>) -> Self {
        if !keep {
            return Self::Drop;
        }
        if let Some(generation) = current {
            return Self::Rotate(generation);
        }
        match Snapshot::open(&dir.prev()) {
            Ok(previous) => Self::Retain(previous.meta.generation),
            Err(_) => Self::Drop,
        }
    }

    fn generation(&self) -> Option<u64> {
        match self {
            Self::Rotate(generation) | Self::Retain(generation) => Some(*generation),
            Self::Drop => None,
        }
    }
}

/// Entries a scan of `tree_roots` will find: inodes in use, once per
/// filesystem. Filesystems that keep no inode count (vfat, exfat) add nothing;
/// an EFI partition holds a few hundred entries next to millions.
pub fn expected_entries(tree_roots: &[PathBuf]) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;
    let mut seen = std::collections::HashSet::new();
    let mut total = None;
    for r in tree_roots {
        let Ok(md) = std::fs::metadata(r) else {
            continue;
        };
        if seen.insert(md.dev())
            && let Some(n) = platform::statvfs(r).ok().and_then(|s| s.inodes_used())
        {
            *total.get_or_insert(0) += n;
        }
    }
    total
}

fn record(dir: &SnapshotDir, op: &str, e: &Error) {
    if matches!(e.code, ErrorCode::Lock | ErrorCode::Usage) {
        return;
    }
    let (op, msg) = match &e.last_error {
        Some(le) => (le.op.clone(), le.message.clone()),
        None => (op.to_string(), e.message.clone()),
    };
    state::record_error(dir, &op, &msg);
}

pub fn run_scan(req: ScanRequest) -> Result<ScanReport> {
    let path_args = scan_root_args(&req.roots, req.one_file_system, req.workers);
    let label = req
        .roots
        .iter()
        .map(|r| r.display().to_string())
        .collect::<Vec<_>>()
        .join(" ");
    fsutil::ensure_dir(req.dir.as_path()).map_err(|e| {
        lock::open_error_with_path_arg("scan", path_args.as_deref(), req.dir.as_path(), e)
    })?;
    let _lock = lock::acquire_with_path_arg(&req.dir, "scan", &label, path_args.as_deref())?;
    let dir = req.dir.clone();
    let r = scan_locked(req, &label);
    if let Err(e) = &r {
        record(&dir, "scan", e);
    }
    r
}

fn scan_locked(req: ScanRequest, label: &str) -> Result<ScanReport> {
    let t_all = Instant::now();
    publish::cleanup_tmp(&req.dir);
    if req.roots.is_empty() {
        return Err(Error::new(ErrorCode::Config, "no roots to scan").with_fix(
            "pass a path (freespace scan /) or set roots in /etc/freespace/config.toml",
        ));
    }
    let seen = Arc::new(AtomicU64::new(0));
    let expected = expected_entries(&req.roots);
    let pw = ProgressWriter::start(
        &req.dir,
        "scan",
        label,
        seen.clone(),
        expected,
        req.on_progress,
    );
    let opts = scan::WalkOptions {
        roots: req.roots.clone(),
        one_file_system: req.one_file_system,
        workers: req.workers,
        seen,
        expected,
    };
    let t = Instant::now();
    let mut tree = scan::walk(&opts).map_err(|e| {
        let code = if e.kind() == std::io::ErrorKind::InvalidInput
            || e.kind() == std::io::ErrorKind::NotFound
        {
            ErrorCode::Usage
        } else {
            ErrorCode::Generic
        };
        Error::new(code, format!("scan failed: {e}"))
    })?;
    let walk_seconds = t.elapsed().as_secs_f64();
    let t = Instant::now();
    let fin = tree.finalize();
    let finalize_seconds = t.elapsed().as_secs_f64();
    let denied = denied_paths(&tree, &fin);
    fault::point(&req.dir, "walked");

    pw.set_stage(progress::STAGE_WRITE);
    let factor = if req.keep_previous { 3 } else { 2 };
    publish::precheck(&req.dir, Some(tree.nodes.len() as u64), factor).map_err(|mut e| {
        e.last_error = Some(state::LastError {
            ts: platform::now_secs(),
            op: "precheck".into(),
            message: e.message.clone(),
        });
        e
    })?;
    fault::point(&req.dir, "prechecked");
    let (generation, current_gen) = next_generation(&req.dir);
    let previous = Previous::for_scan(&req.dir, req.keep_previous, current_gen);
    let prev_generation = previous.generation();
    // State/current may both be unreadable: the retained bytes still establish
    // a lower bound, and diff requires distinct, correctly paired generations.
    let generation = generation.max(prev_generation.map_or(1, |g| g + 1));
    let mut meta = WriteMeta::new(generation, prev_generation, tree.started_at);
    meta.one_file_system = req.one_file_system;
    meta.scan_seconds = Some(walk_seconds);
    let tmp = req.dir.tmp(generation);
    let ws = writer::write_snapshot(&tmp, &tree, &fin, &meta, &Reserve::new(req.dir.as_path()))?;
    fault::point(&req.dir, "written");

    pw.set_stage(progress::STAGE_PUBLISH);
    let t = Instant::now();
    if matches!(previous, Previous::Rotate(_)) {
        publish::rotate_prev(&req.dir)?;
    }
    publish::publish_current(&req.dir, &tmp)?;
    if matches!(previous, Previous::Drop) {
        publish::drop_prev(&req.dir);
    }
    let publish_seconds = t.elapsed().as_secs_f64();
    drop(pw);

    let mut st = state::read(&req.dir).ok().flatten().unwrap_or_default();
    st.last_error = None;
    st.last_success_ts = Some(platform::now_secs());
    st.prev_generation = prev_generation;
    st.current_generation = Some(generation);
    st.warnings = denied_warning(&denied).into_iter().collect();
    st.last_scan_seconds = Some(t_all.elapsed().as_secs_f64());
    state::write(&req.dir, &st).map_err(|e| Error::io("write state.json", e))?;

    let errors = tree
        .roots
        .iter()
        .enumerate()
        .map(|(i, _)| tree.nodes[i].errors as u64)
        .sum();
    Ok(ScanReport {
        generation,
        prev_generation,
        entries: tree.nodes.len() as u64,
        walk_seconds,
        finalize_seconds,
        write: ws,
        publish_seconds,
        total_seconds: t_all.elapsed().as_secs_f64(),
        snapshot_bytes: ws.bytes,
        denied,
        errors,
    })
}

//! `refresh <path>`: rescan one subtree and publish a new generation without
//! rotating prev. The published snapshot is loaded into the same in-memory
//! tree the scanner builds (own values are derived as aggregate − Σ children),
//! the subtree is replaced by a fresh walk, and the scan pipeline runs again:
//! preorder ids, hardlink attribution with the refresh rules (T19, OV-11),
//! the full fold (F2: max/min aggregates are recomputed, never patched), write.

use std::collections::HashMap;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::time::Instant;

use rusqlite::params;

#[path = "refresh_mounts.rs"]
mod mounts;

use crate::error::{Error, ErrorCode, Result};
use crate::job::{denied_paths, denied_warning};
use crate::platform;
use crate::queries::common::{Paths, Row, normalize, resolve, shell_path_word};
use crate::scan::tree::LinkPolicy;
use crate::scan::{self, Kind, MountRow, RootInfo, Tree, flags};
use crate::snapshot::progress::{self, ProgressCallback, ProgressWriter};
use crate::snapshot::reader::Snapshot;
use crate::snapshot::writer::{self, RefreshRow, Reserve, WriteMeta, WriteStats};
use crate::snapshot::{SnapshotDir, lock, publish, state};

pub struct RefreshRequest {
    pub path: Vec<u8>,
    pub workers: usize,
    pub dir: SnapshotDir,
    pub on_progress: Option<ProgressCallback>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RefreshReport {
    pub generation: u64,
    pub path: String,
    pub entries: u64,
    pub subtree_entries: u64,
    pub removed: bool,
    pub load_seconds: f64,
    pub walk_seconds: f64,
    pub finalize_seconds: f64,
    pub write: WriteStats,
    pub publish_seconds: f64,
    pub total_seconds: f64,
    pub snapshot_bytes: u64,
    pub denied: Vec<String>,
}

#[path = "refresh/load.rs"]
mod load;
use load::OldLink;
pub use load::{Loaded, load_tree};

/// Replace the subtree at raw `x` with `sub` (a walk rooted at the same path);
/// `None` = the path is gone on disk and `x` is dropped. Returns the hardlink policy.
pub fn graft(
    loaded: &mut Loaded,
    x_id: i64,
    x_end: i64,
    sub: Option<Tree>,
    conn_paths: &mut Paths<'_>,
) -> Result<LinkPolicy> {
    let tree = &mut loaded.tree;
    let x = loaded.raw_of[x_id as usize];
    // raw id → snapshot id of every dropped descendant
    let mut removed: HashMap<u32, i64> = HashMap::new();
    for id in (x_id + 1)..=x_end {
        let r = loaded.raw_of[id as usize];
        tree.nodes[r as usize].flags |= flags::REMOVED;
        removed.insert(r, id);
    }
    tree.hardlinks.retain(|h| !removed.contains_key(&h.0));
    tree.mounts.retain(|m| !removed.contains_key(&m.raw));

    let new_links = replace_subtree(tree, x, sub);
    hardlink_policy(tree, &loaded.links, &removed, &new_links, conn_paths)
}

fn replace_subtree(tree: &mut Tree, x: u32, sub: Option<Tree>) -> Vec<(u32, u64, u64)> {
    let mut new_links: Vec<(u32, u64, u64)> = Vec::new();
    match sub {
        None => {
            tree.nodes[x as usize].flags |= flags::REMOVED;
            tree.hardlinks.retain(|h| h.0 != x);
            tree.mounts.retain(|m| m.raw != x);
        }
        Some(sub) => {
            let offset = tree.nodes.len() as u32 - 1;
            let names_off = tree.names.len() as u64;
            tree.names.extend_from_slice(&sub.names);
            let map = |r: u32| if r == 0 { x } else { r + offset };
            for (r, nd) in sub.nodes.iter().enumerate().skip(1) {
                let mut n = *nd;
                n.name_off += names_off;
                n.parent = map(n.parent);
                if n.child_count > 0 {
                    n.child_start += offset;
                }
                debug_assert_eq!(tree.nodes.len() as u32, map(r as u32));
                tree.nodes.push(n);
            }
            let root = sub.nodes[0];
            let xn = &mut tree.nodes[x as usize];
            xn.allocated = root.allocated;
            xn.apparent = root.apparent;
            xn.mtime = root.mtime;
            xn.nlink = root.nlink;
            xn.scanned_at = root.scanned_at;
            xn.errors = root.errors;
            xn.flags = root.flags;
            xn.child_count = root.child_count;
            xn.child_start = if root.child_count > 0 {
                root.child_start + offset
            } else {
                0
            };
            for &(r, dev, ino) in &sub.hardlinks {
                new_links.push((map(r), dev, ino));
            }
            tree.hardlinks.extend(new_links.iter().copied());
            for m in sub.mounts {
                tree.mounts.push(MountRow {
                    raw: map(m.raw),
                    ..m
                });
            }
        }
    }

    new_links
}

fn hardlink_policy(
    tree: &Tree,
    old_links: &[OldLink],
    removed: &HashMap<u32, i64>,
    new_links: &[(u32, u64, u64)],
    conn_paths: &mut Paths<'_>,
) -> Result<LinkPolicy> {
    // Hardlink rules (T19, OV-11). Per inode: a previous owner that is still
    // known keeps the size; otherwise count the links we know of (old rows
    // outside the subtree + links seen by this walk). Fewer than `nlink` means
    // an unrecorded link elsewhere carries the size (an earlier OV-11 zeroing
    // or a link made outside since), so nobody here is counted.
    let mut policy = LinkPolicy::default();
    let mut old_by_key: HashMap<(u64, u64), Vec<&OldLink>> = HashMap::new();
    for l in old_links {
        old_by_key.entry((l.1, l.2)).or_default().push(l);
    }
    let mut new_by_key: HashMap<(u64, u64), Vec<u32>> = HashMap::new();
    for &(raw, dev, ino) in new_links {
        new_by_key.entry((dev, ino)).or_default().push(raw);
    }
    let mut keys: Vec<(u64, u64)> = old_by_key
        .keys()
        .chain(new_by_key.keys())
        .copied()
        .collect();
    keys.sort_unstable();
    keys.dedup();
    for key in keys {
        let old = old_by_key.get(&key).map(Vec::as_slice).unwrap_or(&[]);
        let outside: Vec<u32> = old
            .iter()
            .filter(|l| !removed.contains_key(&l.0))
            .map(|l| l.0)
            .collect();
        if !outside.is_empty()
            && let Some(owner) = old.iter().find(|l| l.5)
        {
            match removed.get(&owner.0) {
                None => {
                    policy.forced_raw.insert(key, owner.0);
                }
                Some(&id) => {
                    // owner was inside: keep it if the same path still exists, else smallest remaining
                    policy.forced_path.insert(key, conn_paths.path(id)?);
                }
            }
            continue;
        }
        let new = new_by_key.get(&key).map(Vec::as_slice).unwrap_or(&[]);
        let nlink = new
            .iter()
            .chain(&outside)
            .map(|&r| tree.nodes[r as usize].nlink)
            .max()
            .unwrap_or(0);
        if ((outside.len() + new.len()) as u64) < u64::from(nlink) {
            policy.zeroed.insert(key);
        }
    }
    Ok(policy)
}

pub fn run_refresh(req: RefreshRequest) -> Result<RefreshReport> {
    let path = normalize(&req.path)?;
    let label = String::from_utf8_lossy(&path).into_owned();
    let path_arg = shell_path_word(&path);
    let _lock = lock::acquire_with_path_arg(&req.dir, "refresh", &label, path_arg.as_deref())?;
    let dir = req.dir.clone();
    let r = refresh_locked(req, path, &label);
    if let Err(e) = &r
        && !matches!(
            e.code,
            ErrorCode::Lock | ErrorCode::Usage | ErrorCode::PathNotFound
        )
    {
        let (op, msg) = match &e.last_error {
            Some(le) => (le.op.clone(), le.message.clone()),
            None => ("refresh".to_string(), e.message.clone()),
        };
        state::record_error(&dir, &op, &msg);
    }
    r
}

fn refresh_locked(req: RefreshRequest, path: Vec<u8>, label: &str) -> Result<RefreshReport> {
    let t_all = Instant::now();
    publish::cleanup_tmp(&req.dir);
    let snap = Snapshot::open_current(&req.dir)?;
    let (x, xpath) = resolve(&snap, &path)?;
    check_target(&x, label)?;
    let started = platform::now_secs();
    let seen = Arc::new(AtomicU64::new(0));
    let pw = ProgressWriter::start(
        &req.dir,
        "refresh",
        label,
        seen.clone(),
        Some(x.items as u64),
        req.on_progress,
    );

    let t = Instant::now();
    let mut loaded = load_tree(&snap)?;
    let load_seconds = t.elapsed().as_secs_f64();

    let t = Instant::now();
    let sub = rescan_subtree(
        &snap,
        &loaded.tree.roots,
        &x,
        &xpath,
        req.workers,
        seen,
        label,
    )?;
    let walk_seconds = t.elapsed().as_secs_f64();
    let removed = sub.is_none();
    let subtree_entries = sub.as_ref().map(|t| t.nodes.len() as u64).unwrap_or(0);

    let t = Instant::now();
    let mut paths = Paths::new(&snap.conn);
    let policy = graft(&mut loaded, x.id, x.subtree_end, sub, &mut paths)?;
    drop(paths);
    let mut tree = loaded.tree;
    update_fs_stats(&mut tree, &xpath);
    let fin = tree.finalize_with(&policy);
    let finalize_seconds = t.elapsed().as_secs_f64();
    let denied = denied_paths(&tree, &fin);

    pw.set_stage(progress::STAGE_WRITE);
    precheck(&req.dir, fin.order.len() as u64)?;
    crate::fault::point(&req.dir, "prechecked");
    let (generation, _) = crate::job::next_generation(&req.dir);
    let meta = refresh_meta(&snap, generation, &xpath, started)?;
    let tmp = req.dir.tmp(generation);
    let entries = fin.order.len() as u64;
    let ws = writer::write_snapshot(&tmp, tree, fin, &meta, &Reserve::new(req.dir.as_path()))?;
    drop(snap);
    crate::fault::point(&req.dir, "written");

    pw.set_stage(progress::STAGE_PUBLISH);
    let t = Instant::now();
    publish::publish_current(&req.dir, &tmp)?;
    let publish_seconds = t.elapsed().as_secs_f64();
    drop(pw);

    record_success(&req.dir, generation, &denied)?;

    Ok(RefreshReport {
        generation,
        path: label.to_string(),
        entries,
        subtree_entries,
        removed,
        load_seconds,
        walk_seconds,
        finalize_seconds,
        write: ws,
        publish_seconds,
        total_seconds: t_all.elapsed().as_secs_f64(),
        snapshot_bytes: ws.bytes,
        denied,
    })
}

fn check_target(x: &Row, label: &str) -> Result<()> {
    match x.kind() {
        Kind::Dir => {}
        Kind::Mount => {
            return Err(Error::usage(format!(
                "{label} is a mount point outside this root's filesystem; refresh is per filesystem"
            )));
        }
        _ => return Err(Error::usage(format!("{label} is not a directory"))),
    }
    Ok(())
}

fn subtree_exists(
    os_path: &Path,
    roots: &[RootInfo],
    one_file_system: bool,
    label: &str,
) -> Result<bool> {
    // Only a missing path can remove the cached subtree. Other stat failures
    // (for example, an unsearchable ancestor) must leave the snapshot intact.
    let exists = match os_path.symlink_metadata() {
        Ok(m) if !m.is_dir() => {
            // The parent holds a replacement file or symlink.
            let parent = os_path.parent().unwrap_or(Path::new("/"));
            let fix = match shell_path_word(parent.as_os_str().as_bytes()) {
                Some(parent) => format!("sudo freespace refresh {parent}"),
                None => {
                    "refresh the parent folder with sudo using its original path bytes".to_string()
                }
            };
            return Err(
                Error::usage(format!("{label} is no longer a folder on disk")).with_fix(fix),
            );
        }
        Ok(m) => {
            use std::os::unix::fs::MetadataExt;
            mounts::check(os_path, Some(m.dev()), roots, one_file_system, label)?;
            true
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            mounts::check(os_path, None, roots, one_file_system, label)?;
            false
        }
        Err(e) => return Err(Error::io(format!("stat {label}"), e)),
    };
    Ok(exists)
}

fn rescan_subtree(
    snap: &Snapshot,
    roots: &[RootInfo],
    x: &Row,
    xpath: &[u8],
    workers: usize,
    seen: Arc<AtomicU64>,
    label: &str,
) -> Result<Option<Tree>> {
    let os_path = PathBuf::from(std::ffi::OsStr::from_bytes(xpath));
    if !subtree_exists(&os_path, roots, snap.meta.one_file_system, label)? {
        if x.parent_id.is_none() {
            return Err(Error::usage(format!(
                "root {label} no longer exists; run a full scan"
            )));
        }
        return Ok(None);
    }
    let opts = scan::WalkOptions {
        roots: vec![os_path.clone()],
        one_file_system: snap.meta.one_file_system,
        workers,
        seen,
        expected: Some(x.items as u64 + 1),
    };
    let t = scan::walk(&opts).map_err(|e| Error::generic(format!("refresh walk failed: {e}")))?;
    if t.nodes.is_empty() || t.roots[0].path.as_os_str().as_bytes() != xpath {
        return Err(Error::usage(format!(
            "{label} resolves to {} on disk (symlink?); refresh the real path",
            t.roots
                .first()
                .map(|r| r.path.display().to_string())
                .unwrap_or_default()
        )));
    }
    Ok(Some(t))
}

fn update_fs_stats(tree: &mut Tree, xpath: &[u8]) {
    // a refresh usually follows a cleanup: the free space shown "at scan"
    // should move with it
    if let Some(r) = tree
        .roots
        .iter_mut()
        .filter(|r| is_under(xpath, r.path.as_os_str().as_bytes()))
        .max_by_key(|r| r.path.as_os_str().len())
        && scan::walk::statvfs_safe(&r.fs_type)
        && let Ok(st) = platform::statvfs(&r.path)
    {
        r.stats = st;
    }
}

fn precheck(dir: &SnapshotDir, entries: u64) -> Result<publish::PrecheckInfo> {
    publish::precheck(dir, Some(entries), 2).map_err(|mut e| {
        e.last_error = Some(state::LastError {
            ts: platform::now_secs(),
            op: "precheck".into(),
            message: e.message.clone(),
        });
        e
    })
}

fn refresh_meta(snap: &Snapshot, generation: u64, xpath: &[u8], started: i64) -> Result<WriteMeta> {
    let mut meta = WriteMeta::new(generation, snap.meta.prev_generation, snap.meta.scanned_at);
    meta.one_file_system = snap.meta.one_file_system;
    meta.host = snap.meta.host.clone();
    {
        let mut st = snap
            .conn
            .prepare("SELECT path, scanned_at, generation FROM refreshes")?;
        meta.refreshes = st
            .query_map(params![], |r| {
                Ok(RefreshRow {
                    path: r.get(0)?,
                    scanned_at: r.get(1)?,
                    generation: r.get::<_, i64>(2)? as u64,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
    }
    meta.refreshes.push(RefreshRow {
        path: xpath.to_vec(),
        scanned_at: started,
        generation,
    });
    Ok(meta)
}

fn record_success(dir: &SnapshotDir, generation: u64, denied: &[String]) -> Result<()> {
    let mut st = state::read(dir).ok().flatten().unwrap_or_default();
    st.last_error = None;
    st.last_success_ts = Some(platform::now_secs());
    st.current_generation = Some(generation);
    st.warnings = denied_warning(denied).into_iter().collect();
    state::write(dir, &st).map_err(|e| Error::io("write state.json", e))?;

    Ok(())
}

/// `path` is `root` or lies below it.
fn is_under(path: &[u8], root: &[u8]) -> bool {
    root == b"/" || path == root || (path.starts_with(root) && path.get(root.len()) == Some(&b'/'))
}

#[cfg(test)]
mod tests {
    use super::is_under;

    #[test]
    fn a_path_is_under_its_root_by_whole_names() {
        assert!(is_under(b"/home/tass", b"/"));
        assert!(is_under(b"/home/tass", b"/home"));
        assert!(is_under(b"/home", b"/home"));
        assert!(!is_under(b"/homework", b"/home"));
        assert!(!is_under(b"/var", b"/home"));
    }
}

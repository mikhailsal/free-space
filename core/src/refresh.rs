//! `refresh <path>`: rescan one subtree and publish a new generation without
//! rotating prev. The published snapshot is loaded into the same in-memory
//! tree the scanner builds (own values are derived as aggregate − Σ children),
//! the subtree is replaced by a fresh walk, and the scan pipeline runs again:
//! preorder ids, hardlink attribution with the refresh rules (T19, OV-11),
//! the full fold (F2: max/min aggregates are recomputed, never patched), write.

use std::collections::HashMap;
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::time::Instant;

use rusqlite::params;

use crate::error::{Error, ErrorCode, Result};
use crate::job::{denied_paths, denied_warning};
use crate::platform::{self, FsStats};
use crate::queries::common::{Paths, normalize, resolve};
use crate::scan::tree::LinkPolicy;
use crate::scan::{self, Kind, MountRow, NO_PARENT, Node, RootInfo, Tree, flags};
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

/// Old hardlink row: (raw id, dev, ino, allocated, apparent, was_owner).
type OldLink = (u32, u64, u64, u64, u64, bool);

pub struct Loaded {
    pub tree: Tree,
    /// snapshot id → raw id
    pub raw_of: Vec<u32>,
    pub links: Vec<OldLink>,
}

/// Load a published snapshot into a walker-shaped tree (contiguous, name-ordered
/// child ranges) with own values per node.
pub fn load_tree(snap: &Snapshot) -> Result<Loaded> {
    let conn = &snap.conn;
    let n: i64 = conn.query_row("SELECT count(*) FROM entries", [], |r| r.get(0))?;
    let n = n as usize;
    let mut parent: Vec<u32> = Vec::with_capacity(n);
    let mut child_count = vec![0u32; n];
    {
        let mut st = conn.prepare("SELECT id, parent_id FROM entries ORDER BY id")?;
        let mut rows = st.query([])?;
        while let Some(r) = rows.next()? {
            let id: i64 = r.get(0)?;
            if id as usize != parent.len() {
                return Err(Error::generic("snapshot ids are not dense"));
            }
            let p: Option<i64> = r.get(1)?;
            match p {
                Some(p) => {
                    parent.push(p as u32);
                    child_count[p as usize] += 1;
                }
                None => parent.push(NO_PARENT),
            }
        }
    }
    let roots: Vec<u32> = (0..n as u32)
        .filter(|&i| parent[i as usize] == NO_PARENT)
        .collect();
    let mut block_start = vec![0u32; n];
    let mut next = roots.len() as u32;
    for id in 0..n {
        if child_count[id] > 0 {
            block_start[id] = next;
            next += child_count[id];
        }
    }
    let mut fill = block_start.clone();
    let mut raw_of = vec![0u32; n];
    let mut root_idx = 0u32;
    for id in 0..n {
        let p = parent[id];
        raw_of[id] = if p == NO_PARENT {
            root_idx += 1;
            root_idx - 1
        } else {
            let r = fill[p as usize];
            fill[p as usize] += 1;
            r
        };
    }
    drop(fill);

    let mut tree = Tree {
        started_at: snap.meta.scanned_at,
        ..Default::default()
    };
    tree.nodes.resize(n, Node::default());
    {
        let mut st = conn.prepare(
            "SELECT id, name, kind, flags, allocated, apparent, mtime, nlink, errors, scanned_at FROM entries ORDER BY id",
        )?;
        let mut rows = st.query([])?;
        while let Some(r) = rows.next()? {
            let id = r.get::<_, i64>(0)? as usize;
            let name: Vec<u8> = r.get(1)?;
            let (off, len) = tree.push_name(&name);
            let raw = raw_of[id] as usize;
            let p = parent[id];
            let fl = r.get::<_, i64>(3)? as u8 & !flags::ALIAS;
            tree.nodes[raw] = Node {
                allocated: r.get::<_, i64>(4)? as u64,
                apparent: r.get::<_, i64>(5)? as u64,
                mtime: r.get(6)?,
                scanned_at: r.get(9)?,
                name_off: off,
                name_len: len,
                parent: if p == NO_PARENT {
                    NO_PARENT
                } else {
                    raw_of[p as usize]
                },
                child_start: if child_count[id] > 0 {
                    block_start[id]
                } else {
                    0
                },
                child_count: child_count[id],
                nlink: r.get::<_, i64>(7)? as u32,
                errors: r.get::<_, i64>(8)? as u32,
                kind: r.get::<_, i64>(2)? as u8,
                flags: fl,
                ..Default::default()
            };
        }
    }
    // Own values: subtract each child's aggregate from its parent. In preorder a
    // child is visited before its own children, so it still holds its aggregate.
    for id in 0..n {
        let p = parent[id];
        if p == NO_PARENT {
            continue;
        }
        let c = tree.nodes[raw_of[id] as usize];
        let pn = &mut tree.nodes[raw_of[p as usize] as usize];
        pn.allocated = pn.allocated.saturating_sub(c.allocated);
        pn.apparent = pn.apparent.saturating_sub(c.apparent);
        pn.errors = pn.errors.saturating_sub(c.errors);
    }
    // Hardlinks carry their real sizes (aliases were stored as 0).
    let mut links = Vec::new();
    {
        let mut st = conn.prepare(
            "SELECT h.entry_id, h.dev, h.ino, h.allocated, h.apparent, (e.flags & 2) = 0
             FROM hardlinks h JOIN entries e ON e.id = h.entry_id ORDER BY h.entry_id",
        )?;
        let mut rows = st.query([])?;
        while let Some(r) = rows.next()? {
            let raw = raw_of[r.get::<_, i64>(0)? as usize];
            let (dev, ino) = (r.get::<_, i64>(1)? as u64, r.get::<_, i64>(2)? as u64);
            let (a, ap) = (r.get::<_, i64>(3)? as u64, r.get::<_, i64>(4)? as u64);
            let owner: bool = r.get(5)?;
            let nd = &mut tree.nodes[raw as usize];
            nd.allocated = a;
            nd.apparent = ap;
            tree.hardlinks.push((raw, dev, ino));
            links.push((raw, dev, ino, a, ap, owner));
        }
    }
    {
        let mut st = conn.prepare(
            "SELECT entry_id, path, fs_type, fs_used, fs_total, fs_avail, inodes_used FROM roots ORDER BY entry_id",
        )?;
        let mut rows = st.query([])?;
        while let Some(r) = rows.next()? {
            let path: Vec<u8> = r.get(1)?;
            let used = r.get::<_, i64>(3)? as u64;
            let total = r.get::<_, i64>(4)? as u64;
            let inodes_used: Option<i64> = r.get(6)?;
            tree.roots.push(RootInfo {
                path: PathBuf::from(std::ffi::OsStr::from_bytes(&path)),
                dev: 0,
                fs_type: r.get(2)?,
                stats: FsStats {
                    total,
                    used,
                    free: total.saturating_sub(used),
                    avail: r.get::<_, i64>(5)? as u64,
                    inodes_total: inodes_used.unwrap_or(0) as u64,
                    inodes_free: 0,
                },
            });
        }
    }
    {
        let mut st =
            conn.prepare("SELECT entry_id, path, fs_type, fs_used, fs_total FROM mounts")?;
        let mut rows = st.query([])?;
        while let Some(r) = rows.next()? {
            let path: Vec<u8> = r.get(1)?;
            let used: Option<i64> = r.get(3)?;
            let total: Option<i64> = r.get(4)?;
            tree.mounts.push(MountRow {
                raw: raw_of[r.get::<_, i64>(0)? as usize],
                path: PathBuf::from(std::ffi::OsStr::from_bytes(&path)),
                fs_type: r.get(2)?,
                stats: match (used, total) {
                    (Some(u), Some(t)) => Some(FsStats {
                        used: u as u64,
                        total: t as u64,
                        ..Default::default()
                    }),
                    _ => None,
                },
            });
        }
    }
    Ok(Loaded {
        tree,
        raw_of,
        links,
    })
}

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

    // Hardlink rules (T19, OV-11). Per inode: a previous owner that is still
    // known keeps the size; otherwise count the links we know of (old rows
    // outside the subtree + links seen by this walk). Fewer than `nlink` means
    // an unrecorded link elsewhere carries the size (an earlier OV-11 zeroing
    // or a link made outside since), so nobody here is counted.
    let mut policy = LinkPolicy::default();
    let mut old_by_key: HashMap<(u64, u64), Vec<&OldLink>> = HashMap::new();
    for l in &loaded.links {
        old_by_key.entry((l.1, l.2)).or_default().push(l);
    }
    let mut new_by_key: HashMap<(u64, u64), Vec<u32>> = HashMap::new();
    for &(raw, dev, ino) in &new_links {
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
    let _lock = lock::acquire(&req.dir, "refresh", &label)?;
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
    match x.kind() {
        Kind::Dir => {}
        Kind::Mount => {
            return Err(Error::usage(format!(
                "{label} is a mount point outside this root's filesystem; refresh is per filesystem"
            )));
        }
        _ => return Err(Error::usage(format!("{label} is not a directory"))),
    }
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
    let os_path = PathBuf::from(std::ffi::OsStr::from_bytes(&xpath));
    let sub = if os_path.exists() {
        let opts = scan::WalkOptions {
            roots: vec![os_path.clone()],
            one_file_system: snap.meta.one_file_system,
            workers: req.workers,
            seen,
            expected: Some(x.items as u64 + 1),
        };
        let t =
            scan::walk(&opts).map_err(|e| Error::generic(format!("refresh walk failed: {e}")))?;
        if t.nodes.is_empty() || t.roots[0].path.as_os_str().as_bytes() != xpath.as_slice() {
            return Err(Error::usage(format!(
                "{label} resolves to {} on disk (symlink?); refresh the real path",
                t.roots
                    .first()
                    .map(|r| r.path.display().to_string())
                    .unwrap_or_default()
            )));
        }
        Some(t)
    } else {
        if x.parent_id.is_none() {
            return Err(Error::usage(format!(
                "root {label} no longer exists; run a full scan"
            )));
        }
        None
    };
    let walk_seconds = t.elapsed().as_secs_f64();
    let removed = sub.is_none();
    let subtree_entries = sub.as_ref().map(|t| t.nodes.len() as u64).unwrap_or(0);

    let t = Instant::now();
    let mut paths = Paths::new(&snap.conn);
    let policy = graft(&mut loaded, x.id, x.subtree_end, sub, &mut paths)?;
    drop(paths);
    let mut tree = loaded.tree;
    let fin = tree.finalize_with(&policy);
    let finalize_seconds = t.elapsed().as_secs_f64();
    let denied = denied_paths(&tree, &fin);

    pw.set_stage(progress::STAGE_WRITE);
    publish::precheck(&req.dir, Some(fin.order.len() as u64), 2).map_err(|mut e| {
        e.last_error = Some(state::LastError {
            ts: platform::now_secs(),
            op: "precheck".into(),
            message: e.message.clone(),
        });
        e
    })?;
    crate::fault::point(&req.dir, "prechecked");
    let (generation, _) = crate::job::next_generation(&req.dir);
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
        path: xpath.clone(),
        scanned_at: started,
        generation,
    });
    let tmp = req.dir.tmp(generation);
    let ws = writer::write_snapshot(&tmp, &tree, &fin, &meta, &Reserve::new(req.dir.as_path()))?;
    drop(snap);
    crate::fault::point(&req.dir, "written");

    pw.set_stage(progress::STAGE_PUBLISH);
    let t = Instant::now();
    publish::publish_current(&req.dir, &tmp)?;
    let publish_seconds = t.elapsed().as_secs_f64();
    drop(pw);

    let mut st = state::read(&req.dir).ok().flatten().unwrap_or_default();
    st.last_error = None;
    st.last_success_ts = Some(platform::now_secs());
    st.current_generation = Some(generation);
    st.warnings = denied_warning(&denied).into_iter().collect();
    state::write(&req.dir, &st).map_err(|e| Error::io("write state.json", e))?;

    Ok(RefreshReport {
        generation,
        path: label.to_string(),
        entries: fin.order.len() as u64,
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

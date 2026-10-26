//! Parallel directory walker.
//!
//! N worker threads pull directory jobs from a shared LIFO stack. A worker reads
//! one directory completely (`readdir` + `statx` relative to the directory fd via
//! `DirEntry::metadata`), sorts the entries by name, reserves a contiguous id
//! range for them and sends the batch to the main thread, which owns the tree.
//! A worker holds at most one open directory, so open fds = workers (T3).

use std::collections::HashSet;
use std::fs;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, mpsc};

use super::tree::{Kind, MountRow, NO_PARENT, Node, RootInfo, Tree, flags};
use crate::platform::{self, MountInfo};

#[derive(Debug, Clone)]
pub struct WalkOptions {
    pub roots: Vec<PathBuf>,
    pub one_file_system: bool,
    pub workers: usize,
    /// Entries seen so far, for progress reporting.
    pub seen: Arc<AtomicU64>,
    /// Expected entry count (used inodes), to size the tree up front.
    pub expected: Option<u64>,
}

impl WalkOptions {
    pub fn new(roots: Vec<PathBuf>) -> Self {
        WalkOptions {
            roots,
            one_file_system: true,
            workers: default_workers(),
            seen: Arc::new(AtomicU64::new(0)),
            expected: None,
        }
    }
}

pub fn default_workers() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .clamp(1, 8)
}

struct Job {
    id: u32,
    root: u32,
    path: PathBuf,
}

struct ChildRec {
    name_start: u32,
    name_len: u16,
    kind: u8,
    flags: u8,
    allocated: u64,
    apparent: u64,
    mtime: i64,
    nlink: u32,
    errors: u32,
}

struct DirResult {
    id: u32,
    base: u32,
    scanned_at: i64,
    denied: bool,
    errors: u32,
    names: Vec<u8>,
    children: Vec<ChildRec>,
    hardlinks: Vec<(u32, u64, u64)>,
    mounts: Vec<(u32, PathBuf)>,
}

struct Queue {
    jobs: Vec<Job>,
    /// Jobs pushed and not yet finished (queued + in progress).
    pending: usize,
}

struct RootCtx {
    dev: u64,
    /// Mount points below this root on the same device (bind mounts): `st_dev`
    /// cannot tell them apart, so they are matched by path.
    bind_mounts: HashSet<PathBuf>,
}

struct Ctx {
    queue: Mutex<Queue>,
    cv: Condvar,
    next_id: AtomicU32,
    overflow: AtomicU32,
    seen: Arc<AtomicU64>,
    one_file_system: bool,
    roots: Vec<RootCtx>,
}

/// glibc `dev_t` encoding → "major:minor" as printed by /proc/self/mountinfo.
pub fn dev_string(dev: u64) -> String {
    let major = ((dev >> 8) & 0xfff) | ((dev >> 32) & !0xfff);
    let minor = (dev & 0xff) | ((dev >> 12) & !0xff);
    format!("{major}:{minor}")
}

fn normalize_root(p: &Path) -> io::Result<PathBuf> {
    let c = fs::canonicalize(p)?;
    if !c.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{} is not a directory", p.display()),
        ));
    }
    Ok(c)
}

/// Statvfs only on filesystems where it cannot hang or trigger an automount.
pub fn statvfs_safe(fs_type: &str) -> bool {
    platform::REAL_FS.contains(&fs_type)
        || matches!(
            fs_type,
            "tmpfs" | "devtmpfs" | "overlay" | "squashfs" | "ramfs"
        )
}

pub fn walk(opts: &WalkOptions) -> io::Result<Tree> {
    let mounts = platform::visible_mountinfo();
    walk_with_mounts(opts, &mounts)
}

pub fn walk_with_mounts(opts: &WalkOptions, mounts: &[MountInfo]) -> io::Result<Tree> {
    let limit = platform::raise_nofile_limit();
    let workers = opts
        .workers
        .clamp(1, (limit.saturating_sub(64)).max(1) as usize);
    let started_at = platform::now_secs();

    let mut tree = Tree {
        started_at,
        ..Default::default()
    };
    let (root_paths, root_ctx) = prepare_roots(opts, mounts, started_at, &mut tree)?;

    let jobs: Vec<Job> = root_paths
        .iter()
        .enumerate()
        .rev()
        .map(|(i, p)| Job {
            id: i as u32,
            root: i as u32,
            path: p.clone(),
        })
        .collect();
    let ctx = Ctx {
        queue: Mutex::new(Queue {
            pending: jobs.len(),
            jobs,
        }),
        cv: Condvar::new(),
        next_id: AtomicU32::new(root_paths.len() as u32),
        overflow: AtomicU32::new(0),
        seen: opts.seen.clone(),
        one_file_system: opts.one_file_system,
        roots: root_ctx,
    };
    opts.seen
        .fetch_add(root_paths.len() as u64, Ordering::Relaxed);

    let (tx, rx) = mpsc::channel::<DirResult>();
    std::thread::scope(|s| {
        for _ in 0..workers {
            let tx = tx.clone();
            let ctx = &ctx;
            s.spawn(move || worker(ctx, tx));
        }
        drop(tx);
        for res in rx {
            apply(&mut tree, res);
        }
    });
    if ctx.overflow.load(Ordering::Relaxed) != 0 {
        return Err(io::Error::other("more than 4 billion entries"));
    }
    finish_mounts(&mut tree, mounts);
    Ok(tree)
}

fn prepare_roots(
    opts: &WalkOptions,
    mounts: &[MountInfo],
    started_at: i64,
    tree: &mut Tree,
) -> io::Result<(Vec<PathBuf>, Vec<RootCtx>)> {
    if let Some(expected) = opts.expected {
        // +2% headroom: entries on other filesystems' mount nodes, files created meanwhile
        let n = (expected + expected / 50).min(u32::MAX as u64) as usize;
        tree.nodes.reserve_exact(n);
        tree.names.reserve_exact(n.saturating_mul(16));
    }

    let mut root_paths = Vec::new();
    for root in &opts.roots {
        let path = normalize_root(root)?;
        if !root_paths.contains(&path) {
            root_paths.push(path);
        }
    }

    let mut root_ctx = Vec::new();
    for path in &root_paths {
        let metadata = fs::metadata(path)?;
        validate_root_nesting(path, &root_paths, metadata.dev())?;
        let dev_s = dev_string(metadata.dev());
        let bind_mounts = platform::bind_mounts_below(mounts, path, &dev_s)
            .into_iter()
            .collect();
        let fs_type = platform::mount_for(mounts, path)
            .map(|mount| mount.fs_type)
            .unwrap_or_default();
        let (off, len) = tree.push_name(path.as_os_str().as_bytes());
        tree.nodes.push(Node {
            allocated: metadata.blocks() * 512,
            apparent: metadata.len(),
            mtime: metadata.mtime(),
            scanned_at: started_at,
            name_off: off,
            name_len: len,
            parent: NO_PARENT,
            nlink: metadata.nlink().min(u32::MAX as u64) as u32,
            kind: Kind::Dir.code(),
            ..Default::default()
        });
        tree.roots.push(RootInfo {
            path: path.clone(),
            dev: metadata.dev(),
            stats: platform::statvfs(path).unwrap_or_default(),
            fs_type,
        });
        root_ctx.push(RootCtx {
            dev: metadata.dev(),
            bind_mounts,
        });
    }
    Ok((root_paths, root_ctx))
}

fn validate_root_nesting(path: &Path, roots: &[PathBuf], dev: u64) -> io::Result<()> {
    for other in roots {
        if other != path && path.starts_with(other) && fs::metadata(other)?.dev() == dev {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "root {} is inside root {} on the same filesystem",
                    path.display(),
                    other.display()
                ),
            ));
        }
    }
    Ok(())
}

fn finish_mounts(tree: &mut Tree, mounts: &[MountInfo]) {
    for m in tree.mounts.iter_mut() {
        let info = mounts.iter().rev().find(|mi| mi.mount_point == m.path);
        m.fs_type = info.map(|i| i.fs_type.clone()).unwrap_or_default();
        if statvfs_safe(&m.fs_type) {
            m.stats = platform::statvfs(&m.path).ok();
        }
    }
}

fn worker(ctx: &Ctx, tx: mpsc::Sender<DirResult>) {
    loop {
        let job = {
            let mut q = ctx.queue.lock().unwrap();
            loop {
                if let Some(j) = q.jobs.pop() {
                    break Some(j);
                }
                if q.pending == 0 {
                    break None;
                }
                q = ctx.cv.wait(q).unwrap();
            }
        };
        let Some(job) = job else {
            ctx.cv.notify_all();
            return;
        };
        let (res, sub) = read::read_dir(ctx, &job);
        let _ = tx.send(res);
        let mut q = ctx.queue.lock().unwrap();
        let k = sub.len();
        q.jobs.extend(sub);
        q.pending = q.pending + k - 1;
        let done = q.pending == 0;
        drop(q);
        if done {
            ctx.cv.notify_all();
        } else if k > 0 {
            if k == 1 {
                ctx.cv.notify_one();
            } else {
                ctx.cv.notify_all();
            }
        }
    }
}

fn apply(tree: &mut Tree, res: DirResult) {
    let n = res.children.len();
    let need = res.base as usize + n;
    if tree.nodes.len() < need {
        tree.nodes.resize(need, Node::default());
    }
    {
        let dir = &mut tree.nodes[res.id as usize];
        dir.child_start = res.base;
        dir.child_count = n as u32;
        dir.scanned_at = dir.scanned_at.max(res.scanned_at);
        dir.errors += res.errors;
        if res.denied {
            dir.flags |= flags::DENIED;
        }
    }
    for (i, c) in res.children.iter().enumerate() {
        let name = &res.names[c.name_start as usize..c.name_start as usize + c.name_len as usize];
        let off = tree.names.len() as u64;
        tree.names.extend_from_slice(name);
        let nd = &mut tree.nodes[res.base as usize + i];
        nd.allocated = c.allocated;
        nd.apparent = c.apparent;
        nd.mtime = c.mtime;
        nd.scanned_at = nd.scanned_at.max(res.scanned_at);
        nd.name_off = off;
        nd.name_len = c.name_len;
        nd.parent = res.id;
        nd.nlink = c.nlink;
        nd.errors += c.errors;
        nd.kind = c.kind;
        nd.flags |= c.flags;
    }
    tree.hardlinks.extend(res.hardlinks);
    for (id, path) in res.mounts {
        tree.mounts.push(MountRow {
            raw: id,
            path,
            fs_type: String::new(),
            stats: None,
        });
    }
}

mod read;

#[cfg(test)]
#[path = "walk/tests.rs"]
mod tests;

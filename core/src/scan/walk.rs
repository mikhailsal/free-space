//! Parallel directory walker.
//!
//! N worker threads pull directory jobs from a shared LIFO stack. A worker reads
//! one directory completely (`readdir` + `statx` relative to the directory fd via
//! `DirEntry::metadata`), sorts the entries by name, reserves a contiguous id
//! range for them and sends the batch to the main thread, which owns the tree.
//! A worker holds at most one open directory, so open fds = workers (T3).

use std::collections::HashSet;
use std::ffi::OsString;
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
    let mounts = platform::mountinfo();
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
    if let Some(e) = opts.expected {
        // +2% headroom: entries on other filesystems' mount nodes, files created meanwhile
        let n = (e + e / 50).min(u32::MAX as u64) as usize;
        tree.nodes.reserve_exact(n);
        tree.names.reserve_exact(n.saturating_mul(16));
    }
    let mut root_ctx = Vec::new();
    let mut root_paths: Vec<PathBuf> = Vec::new();
    for r in &opts.roots {
        let p = normalize_root(r)?;
        if root_paths.contains(&p) {
            continue;
        }
        root_paths.push(p);
    }
    for p in &root_paths {
        let md = fs::metadata(p)?;
        for q in &root_paths {
            if q != p && p.starts_with(q) && fs::metadata(q)?.dev() == md.dev() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!(
                        "root {} is inside root {} on the same filesystem",
                        p.display(),
                        q.display()
                    ),
                ));
            }
        }
        let dev_s = dev_string(md.dev());
        let bind_mounts = mounts
            .iter()
            .filter(|m| m.mount_point != *p && m.mount_point.starts_with(p) && m.dev == dev_s)
            .map(|m| m.mount_point.clone())
            .collect();
        let fs_type = platform::mount_for(mounts, p)
            .map(|m| m.fs_type)
            .unwrap_or_default();
        let (off, len) = tree.push_name(p.as_os_str().as_bytes());
        tree.nodes.push(Node {
            allocated: md.blocks() * 512,
            apparent: md.len(),
            mtime: md.mtime(),
            scanned_at: started_at,
            name_off: off,
            name_len: len,
            parent: NO_PARENT,
            nlink: md.nlink().min(u32::MAX as u64) as u32,
            kind: Kind::Dir.code(),
            ..Default::default()
        });
        tree.roots.push(RootInfo {
            path: p.clone(),
            dev: md.dev(),
            stats: platform::statvfs(p).unwrap_or_default(),
            fs_type,
        });
        root_ctx.push(RootCtx {
            dev: md.dev(),
            bind_mounts,
        });
    }

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
        let (res, sub) = read_dir(ctx, &job);
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

fn read_dir(ctx: &Ctx, job: &Job) -> (DirResult, Vec<Job>) {
    let mut res = DirResult {
        id: job.id,
        base: 0,
        scanned_at: platform::now_secs(),
        denied: false,
        errors: 0,
        names: Vec::new(),
        children: Vec::new(),
        hardlinks: Vec::new(),
        mounts: Vec::new(),
    };
    let rd = match fs::read_dir(&job.path) {
        Ok(rd) => rd,
        Err(_) => {
            res.denied = true;
            res.errors = 1;
            return (res, Vec::new());
        }
    };
    let mut ents: Vec<(OsString, io::Result<fs::Metadata>, Option<fs::FileType>)> = Vec::new();
    for e in rd {
        match e {
            Ok(e) => {
                let md = e.metadata();
                let ft = if md.is_err() {
                    e.file_type().ok()
                } else {
                    None
                };
                ents.push((e.file_name(), md, ft));
            }
            Err(_) => {
                // readdir failed mid-stream: the listing is incomplete.
                res.denied = true;
                res.errors += 1;
                break;
            }
        }
    }
    ents.sort_unstable_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));

    let n = ents.len() as u32;
    let base = ctx.next_id.fetch_add(n, Ordering::Relaxed);
    if base.checked_add(n).is_none_or(|e| e == u32::MAX) {
        ctx.overflow.store(1, Ordering::Relaxed);
        res.denied = true;
        return (res, Vec::new());
    }
    res.base = base;
    ctx.seen.fetch_add(n as u64, Ordering::Relaxed);
    let root = &ctx.roots[job.root as usize];
    let mut sub = Vec::new();
    res.children.reserve(ents.len());
    for (i, (name, md, ft)) in ents.into_iter().enumerate() {
        let id = base + i as u32;
        let nb = name.as_bytes();
        let name_start = res.names.len() as u32;
        let name_len = nb.len().min(u16::MAX as usize) as u16;
        res.names.extend_from_slice(&nb[..name_len as usize]);
        let mut rec = ChildRec {
            name_start,
            name_len,
            kind: Kind::Other.code(),
            flags: 0,
            allocated: 0,
            apparent: 0,
            mtime: 0,
            nlink: 1,
            errors: 0,
        };
        match md {
            Ok(md) => {
                let t = md.file_type();
                let mut kind = if t.is_dir() {
                    Kind::Dir
                } else if t.is_file() {
                    Kind::File
                } else if t.is_symlink() {
                    Kind::Symlink
                } else {
                    Kind::Other
                };
                rec.allocated = md.blocks() * 512;
                rec.apparent = md.len();
                rec.mtime = md.mtime();
                rec.nlink = md.nlink().min(u32::MAX as u64) as u32;
                if kind == Kind::Dir {
                    let child_path = job.path.join(&name);
                    let crosses = md.dev() != root.dev;
                    if (crosses && ctx.one_file_system) || root.bind_mounts.contains(&child_path) {
                        kind = Kind::Mount;
                        rec.allocated = 0;
                        rec.apparent = 0;
                        res.mounts.push((id, child_path));
                    } else {
                        sub.push(Job {
                            id,
                            root: job.root,
                            path: child_path,
                        });
                    }
                } else if md.nlink() > 1 {
                    res.hardlinks.push((id, md.dev(), md.ino()));
                }
                rec.kind = kind.code();
            }
            Err(_) => {
                let kind = match ft {
                    Some(t) if t.is_dir() => Kind::Dir,
                    Some(t) if t.is_file() => Kind::File,
                    Some(t) if t.is_symlink() => Kind::Symlink,
                    Some(t) if t.is_fifo() || t.is_socket() => Kind::Other,
                    _ => Kind::Other,
                };
                rec.kind = kind.code();
                rec.flags |= flags::STAT_ERROR;
                if kind == Kind::Dir {
                    rec.flags |= flags::DENIED;
                }
                rec.errors = 1;
            }
        }
        res.children.push(rec);
    }
    // LIFO stack pops the last job first: push in reverse so siblings run in name order.
    sub.reverse();
    (res, sub)
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};

    fn walk_dir(p: &Path, workers: usize) -> Tree {
        let mut o = WalkOptions::new(vec![p.to_path_buf()]);
        o.workers = workers;
        walk_with_mounts(&o, &[]).unwrap()
    }

    fn names_preorder(t: &mut Tree) -> Vec<String> {
        let f = t.finalize();
        f.order
            .iter()
            .map(|&r| String::from_utf8_lossy(t.name(&t.nodes[r as usize])).into_owned())
            .collect()
    }

    #[test]
    fn walks_sorted_and_counts() {
        let d = tempfile::tempdir().unwrap();
        fs::create_dir_all(d.path().join("b/c")).unwrap();
        fs::write(d.path().join("b/c/f1"), vec![1u8; 10_000]).unwrap();
        fs::write(d.path().join("a.txt"), b"hello").unwrap();
        symlink("/nonexistent", d.path().join("zlink")).unwrap();
        for w in [1, 4] {
            let mut t = walk_dir(d.path(), w);
            let names = names_preorder(&mut t);
            assert_eq!(&names[1..], &["a.txt", "b", "c", "f1", "zlink"]);
            let root = t.nodes[0];
            assert_eq!(root.items, 5);
            assert!(root.apparent >= 10_005);
            assert_eq!(root.errors, 0);
        }
    }

    #[test]
    fn denied_dir_is_flagged() {
        if unsafe { libc::geteuid() } == 0 {
            return; // root reads everything
        }
        let d = tempfile::tempdir().unwrap();
        let locked = d.path().join("locked");
        fs::create_dir(&locked).unwrap();
        fs::write(locked.join("secret"), b"x").unwrap();
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
        let mut t = walk_dir(d.path(), 2);
        let f = t.finalize();
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(f.denied.len(), 1);
        assert_eq!(t.nodes[0].errors, 1);
    }

    #[test]
    fn wide_fanout_under_low_nofile() {
        // T3: 20k-wide fan-out must not hit EMFILE; open fds stay at `workers`.
        let d = tempfile::tempdir().unwrap();
        for i in 0..20_000 {
            fs::create_dir(d.path().join(format!("d{i:05}"))).unwrap();
        }
        let mut t = walk_dir(d.path(), 8);
        t.finalize();
        assert_eq!(t.nodes[0].items, 20_000);
        assert_eq!(t.nodes[0].errors, 0);
    }

    #[test]
    fn hardlinks_counted_once() {
        let d = tempfile::tempdir().unwrap();
        fs::create_dir(d.path().join("x")).unwrap();
        fs::write(d.path().join("x/big"), vec![7u8; 1 << 20]).unwrap();
        fs::hard_link(d.path().join("x/big"), d.path().join("link")).unwrap();
        let mut t = walk_dir(d.path(), 3);
        let f = t.finalize();
        assert_eq!(f.hardlinks.len(), 2);
        let total = t.nodes[0].allocated;
        let x = t.nodes.iter().find(|n| t.name(n) == b"x").unwrap();
        // "/…/link" < "/…/x/big": the top-level link carries the size.
        assert!(x.allocated < 1 << 20);
        assert!((1 << 20..2 << 20).contains(&total));
    }

    #[test]
    fn dev_string_matches_mountinfo_format() {
        assert_eq!(dev_string(0x803), "8:3");
        assert_eq!(dev_string(0x16), "0:22");
    }
}

use crate::buckets::{Slots, slot_of, tile};
use crate::model::{BUCKET_ENTRIES, Bucket, MAX_DEPTH, MAX_RECORDS, Node, R, SLOTS, Stats, now};
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::{fs, io};

type Owners = HashMap<(u64, u64), PathBuf>;

/// Limits a scan of one retained directory to the folded children whose name hash falls in
/// `[lo, hi)`. Children that are retained nodes in the snapshot are skipped by name.
pub struct Scope {
    pub lo: u32,
    pub hi: u32,
    pub skip: HashSet<Vec<u8>>,
}
impl Scope {
    fn admits(&self, name: &[u8]) -> bool {
        let slot = slot_of(name);
        self.lo <= slot && slot < self.hi && !self.skip.contains(name)
    }
}
fn admitted(scope: Option<&Scope>, name: &std::ffi::OsStr) -> bool {
    scope.is_none_or(|s| s.admits(name.as_bytes()))
}

#[derive(Default, PartialEq, Eq)]
struct Digest {
    count: u64,
    xor: u64,
    sum: u64,
}
impl Digest {
    fn add(&mut self, p: &Path, m: &fs::Metadata) {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (
            p.as_os_str().as_bytes(),
            m.dev(),
            m.ino(),
            m.mode(),
            m.nlink(),
            m.len(),
            m.blocks(),
            m.mtime(),
            m.mtime_nsec(),
        )
            .hash(&mut h);
        let v = h.finish();
        self.count += 1;
        self.xor ^= v;
        self.sum = self.sum.wrapping_add(v);
    }
}
struct Frame {
    node: Node,
    iter: fs::ReadDir,
    /// Slot of this directory's name inside its parent; where it lands if it is folded away.
    slot: u32,
    /// Folded children of this directory, accumulated per name-hash slot.
    slots: Slots,
}
struct Folded {
    retained: Vec<Node>,
    root: Node,
    slots: Slots,
    digest: Digest,
}
pub struct ResultTree {
    pub nodes: Vec<Node>,
    /// Residual buckets of the scanned directory when only a slot range was scanned.
    pub buckets: Vec<Bucket>,
    pub visited: u64,
    pub has_hardlinks: bool,
}

fn inspect(path: &Path, root_dev: u64, mounts: &HashSet<PathBuf>) -> R<fs::Metadata> {
    if mounts.contains(path) {
        return Err(format!("nested mount: {}", path.display()).into());
    }
    let m = fs::symlink_metadata(path)?;
    if m.dev() != root_dev {
        return Err(format!("cross-device path: {}", path.display()).into());
    }
    Ok(m)
}
fn record_owner(owners: &mut Owners, path: &Path, metadata: &fs::Metadata) -> R<()> {
    owners
        .entry((metadata.dev(), metadata.ino()))
        .and_modify(|old| {
            if path.as_os_str().as_bytes() < old.as_os_str().as_bytes() {
                *old = path.to_path_buf();
            }
        })
        .or_insert_with(|| path.to_path_buf());
    if owners.len() > MAX_RECORDS {
        return Err("hardlink owner cap exceeded".into());
    }
    Ok(())
}
fn retain(retained: &mut Vec<Node>, node: Node) -> R<()> {
    if retained.len() >= MAX_RECORDS {
        return Err("retained record cap exceeded".into());
    }
    retained.push(node);
    Ok(())
}
fn metadata_pass(
    root: &Path,
    dev: u64,
    mounts: &HashSet<PathBuf>,
    scope: Option<&Scope>,
) -> R<(Owners, Digest)> {
    let mut owners = HashMap::new();
    let mut digest = Digest::default();
    let meta = inspect(root, dev, mounts)?;
    digest.add(root, &meta);
    let mut stack = vec![fs::read_dir(root)?];
    while let Some(top) = stack.last_mut() {
        let Some(entry) = top.next() else {
            stack.pop();
            continue;
        };
        let entry = entry?;
        if stack.len() == 1 && !admitted(scope, &entry.file_name()) {
            continue;
        }
        let p = entry.path();
        let m = inspect(&p, dev, mounts)?;
        digest.add(&p, &m);
        if digest.count > 10_000_000 {
            return Err("visited entry cap exceeded".into());
        }
        if !m.is_dir() && m.nlink() > 1 {
            record_owner(&mut owners, &p, &m)?;
        }
        if m.is_dir() {
            if stack.len() >= MAX_DEPTH {
                return Err("directory depth limit exceeded".into());
            }
            stack.push(fs::read_dir(&p)?);
        }
    }
    Ok((owners, digest))
}
fn node(
    path: PathBuf,
    parent: Option<PathBuf>,
    m: &fs::Metadata,
    owners: &Owners,
    stamp: i64,
) -> Node {
    let kind = if m.is_dir() {
        1
    } else if m.is_file() {
        0
    } else {
        2
    };
    let linked = kind != 1 && m.nlink() > 1;
    let alias = linked && owners.get(&(m.dev(), m.ino())) != Some(&path);
    let (allocated, apparent) = if alias {
        (0, 0)
    } else {
        (m.blocks().saturating_mul(512), m.len())
    };
    let total = Stats {
        allocated,
        apparent,
        files: u64::from(kind == 0),
        dirs: u64::from(kind == 1),
        others: u64::from(kind == 2),
        errors: 0,
        links: u64::from(linked),
        min_scanned_at: Some(stamp),
        newest_mtime: if kind == 0 {
            Some(m.mtime().min(stamp))
        } else {
            None
        },
    };
    Node {
        path,
        parent,
        kind,
        own_allocated: allocated,
        own_apparent: apparent,
        own_errors: 0,
        total,
        scanned_at: stamp,
        buckets: Vec::new(),
    }
}
/// Give a directory its residual buckets; every retained directory covers all slots.
fn seal(mut node: Node, slots: &Slots) -> R<Node> {
    node.buckets = tile(slots, BUCKET_ENTRIES, 0, SLOTS)?;
    Ok(node)
}
/// A directory is retained for its size, or because folding it would make a refresh of
/// anything below it as expensive as rescanning everything below it.
pub fn keep(total: &Stats, threshold: u64) -> bool {
    total.allocated >= threshold || total.entries() > BUCKET_ENTRIES
}
fn finish(stack: &mut Vec<Frame>, retained: &mut Vec<Node>, threshold: u64) -> R<()> {
    let frame = stack.pop().ok_or("empty stack")?;
    let parent = stack.last_mut().ok_or("missing parent frame")?;
    parent.node.total.add(&frame.node.total)?;
    if keep(&frame.node.total, threshold) {
        retain(retained, seal(frame.node, &frame.slots)?)
    } else {
        parent
            .slots
            .entry(frame.slot)
            .or_default()
            .add(&frame.node.total)
    }
}
fn fold_pass(
    root: &Path,
    dev: u64,
    threshold: u64,
    owners: &Owners,
    mounts: &HashSet<PathBuf>,
    scope: Option<&Scope>,
) -> R<Folded> {
    let mut retained = Vec::new();
    let mut digest = Digest::default();
    let stamp = now();
    let m = inspect(root, dev, mounts)?;
    digest.add(root, &m);
    let mut stack = vec![Frame {
        node: node(root.to_path_buf(), None, &m, owners, stamp),
        iter: fs::read_dir(root)?,
        slot: 0,
        slots: Slots::new(),
    }];
    loop {
        let depth = stack.len();
        let Some(frame) = stack.last_mut() else {
            break;
        };
        let entry = match frame.iter.next() {
            Some(r) => r?,
            None if depth > 1 => {
                finish(&mut stack, &mut retained, threshold)?;
                continue;
            }
            None => break,
        };
        let name = entry.file_name();
        if depth == 1 && !admitted(scope, &name) {
            continue;
        }
        let p = entry.path();
        let m = inspect(&p, dev, mounts)?;
        digest.add(&p, &m);
        if digest.count > 10_000_000 {
            return Err("visited entry cap exceeded".into());
        }
        let slot = slot_of(name.as_bytes());
        let n = node(p.clone(), Some(frame.node.path.clone()), &m, owners, stamp);
        if m.is_dir() {
            if depth >= MAX_DEPTH {
                return Err("directory depth limit exceeded".into());
            }
            stack.push(Frame {
                node: n,
                iter: fs::read_dir(p)?,
                slot,
                slots: Slots::new(),
            });
        } else {
            frame.node.total.add(&n.total)?;
            if n.total.allocated >= threshold {
                retain(&mut retained, n)?;
            } else {
                frame.slots.entry(slot).or_default().add(&n.total)?;
            }
        }
    }
    let top = stack.pop().ok_or("empty stack")?;
    Ok(Folded {
        retained,
        root: top.node,
        slots: top.slots,
        digest,
    })
}
fn scan_inner(root: &Path, threshold: u64, scope: Option<&Scope>) -> R<ResultTree> {
    let meta = fs::symlink_metadata(root)?;
    if !meta.is_dir() {
        return Err("root must be a directory".into());
    }
    let mounts = crate::mounts::nested(root)?;
    let (owners, first) = metadata_pass(root, meta.dev(), &mounts, scope)?;
    if crate::mounts::nested(root)? != mounts {
        return Err("mount topology changed".into());
    }
    let folded = fold_pass(root, meta.dev(), threshold, &owners, &mounts, scope)?;
    if crate::mounts::nested(root)? != mounts {
        return Err("mount topology changed".into());
    }
    if first != folded.digest {
        return Err(io::Error::other("filesystem changed between metadata passes").into());
    }
    let visited = first.count + folded.digest.count;
    let mut nodes = folded.retained;
    let buckets = match scope {
        None => {
            nodes.push(seal(folded.root, &folded.slots)?);
            Vec::new()
        }
        Some(s) => tile(&folded.slots, BUCKET_ENTRIES, s.lo, s.hi)?,
    };
    Ok(ResultTree {
        nodes,
        buckets,
        visited,
        has_hardlinks: !owners.is_empty(),
    })
}
/// Scan a whole retained directory (the filesystem root of a snapshot, or a retained
/// refresh boundary) and fold everything small below it into buckets.
pub fn scan(root: &Path, threshold: u64) -> R<ResultTree> {
    scan_inner(root, threshold, None)
}
/// Rescan only the folded children of `parent` that hash into the scope's slot range.
/// The result's buckets re-tile exactly that range.
pub fn scan_range(parent: &Path, threshold: u64, scope: &Scope) -> R<ResultTree> {
    scan_inner(parent, threshold, Some(scope))
}

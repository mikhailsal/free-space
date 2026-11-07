//! Two-pass walk of one directory (or of one slot range of it). The first pass finds
//! which path owns the bytes of every hardlinked inode; the second folds everything
//! small into per-directory buckets and keeps the rest as nodes. Directories are read
//! as a stream, so memory follows the depth of the tree and the number of retained
//! entries, never its size.

use std::ffi::OsStr;
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use super::buckets::{Slots, slot_of, tile};
use super::model::{BUCKET_ENTRIES, MAX_DEPTH, MAX_NODES, Node, SLOTS, Stats, invariant, large};
use crate::error::{Error, Result};
use crate::platform;
use crate::scan::{Kind, flags};

mod links;
mod types;

pub use links::{Owners, link_owners};
pub use types::{Boundary, Scope, Walk, Walked};

struct Frame {
    node: Node,
    iter: fs::ReadDir,
    /// Slot of this directory's name inside its parent; where it lands if it is folded away.
    slot: u32,
    /// Folded children of this directory, accumulated per name-hash slot.
    slots: Slots,
    /// Some child of this directory is retained, so the directory must be as well.
    kept_child: bool,
}

/// The walked directory itself once its listing is over.
struct Top {
    node: Node,
    slots: Slots,
}

struct Fold<'a, 'w> {
    w: &'a Walk<'w>,
    owners: &'a mut Owners,
    stack: Vec<Frame>,
    retained: Vec<Node>,
    top: Option<Top>,
    visited: u64,
}

fn classify(t: fs::FileType) -> Kind {
    if t.is_dir() {
        Kind::Dir
    } else if t.is_file() {
        Kind::File
    } else if t.is_symlink() {
        Kind::Symlink
    } else {
        Kind::Other
    }
}

fn blank(kind: Kind, stamp: i64) -> Node {
    Node {
        path: PathBuf::new(),
        root: false,
        kind,
        flags: 0,
        nlink: 1,
        mtime: 0,
        scanned_at: stamp,
        own_allocated: 0,
        own_apparent: 0,
        own_errors: 0,
        total: Stats::default(),
        buckets: Vec::new(),
        mount: None,
    }
}

/// An entry described by its `lstat`, without a path: only entries that are kept need one.
fn described(kind: Kind, md: &fs::Metadata, stamp: i64) -> Node {
    Node {
        nlink: md.nlink().min(u64::from(u32::MAX)) as u32,
        mtime: md.mtime(),
        own_allocated: md.blocks().saturating_mul(512),
        own_apparent: md.len(),
        ..blank(kind, stamp)
    }
}

fn too_many() -> Error {
    Error::usage(format!(
        "more than {MAX_NODES} entries are large enough to keep; raise the folding threshold (--coarse-ppm, coarse_ppm)"
    ))
}

impl Fold<'_, '_> {
    fn frame(&mut self) -> Result<&mut Frame> {
        self.stack
            .last_mut()
            .ok_or_else(|| invariant("walk stack is empty"))
    }

    /// Account for a finished entry in its parent and either keep or fold it. `name` is
    /// the entry's name below the directory being read; the path is built only for
    /// entries that stay.
    fn place(&mut self, mut node: Node, name: &OsStr, slot: u32) -> Result<()> {
        node.total = node.own();
        let threshold = self.w.threshold;
        let frame = self.frame()?;
        frame.node.total.add(&node.total);
        if node.total.allocated < threshold && !node.pinned() {
            frame.slots.entry(slot).or_default().add(&node.total);
            return Ok(());
        }
        frame.kept_child = true;
        if node.path.as_os_str().is_empty() {
            node.path = frame.node.path.join(name);
        }
        if node.is_dir() {
            node.buckets = tile(&Slots::new(), BUCKET_ENTRIES, 0, SLOTS)?;
        }
        self.retain(node)
    }

    fn retain(&mut self, node: Node) -> Result<()> {
        if self.retained.len() >= MAX_NODES {
            return Err(too_many());
        }
        self.retained.push(node);
        Ok(())
    }

    fn entry(&mut self, entry: &fs::DirEntry) -> Result<()> {
        let name = entry.file_name();
        if self.stack.len() == 1 && !self.w.admits(&name) {
            return Ok(());
        }
        self.visited += 1;
        self.w.seen.fetch_add(1, Ordering::Relaxed);
        let slot = slot_of(name.as_bytes());
        let stamp = self.frame()?.node.scanned_at;
        match entry.metadata() {
            Ok(md) if md.is_dir() => self.directory(&md, &name, slot, stamp),
            Ok(md) => self.leaf(&md, &name, slot, stamp),
            Err(_) => {
                let kind = entry.file_type().map_or(Kind::Other, classify);
                let mut node = blank(kind, stamp);
                node.flags = flags::STAT_ERROR;
                node.own_errors = 1;
                if kind == Kind::Dir {
                    node.flags |= flags::DENIED;
                }
                self.place(node, &name, slot)
            }
        }
    }

    fn directory(&mut self, md: &fs::Metadata, name: &OsStr, slot: u32, stamp: i64) -> Result<()> {
        let path = self.frame()?.node.path.join(name);
        let mut node = described(Kind::Dir, md, stamp);
        if self.w.boundary.is_mount(md, &path) {
            node.kind = Kind::Mount;
            node.own_allocated = 0;
            node.own_apparent = 0;
            node.mount = Some(Box::new(self.w.boundary.mount_meta(&path)));
            node.path = path;
            return self.place(node, name, slot);
        }
        node.path = path;
        node.scanned_at = platform::now_secs();
        match fs::read_dir(&node.path) {
            Ok(iter) if self.stack.len() < MAX_DEPTH => {
                node.total = node.own();
                self.stack.push(Frame {
                    node,
                    iter,
                    slot,
                    slots: Slots::new(),
                    kept_child: false,
                });
                Ok(())
            }
            _ => {
                // A directory that cannot be listed stays in the snapshot as an unreadable leaf.
                node.flags |= flags::DENIED;
                node.own_errors += 1;
                self.place(node, name, slot)
            }
        }
    }

    fn leaf(&mut self, md: &fs::Metadata, name: &OsStr, slot: u32, stamp: i64) -> Result<()> {
        let mut node = described(classify(md.file_type()), md, stamp);
        if md.nlink() > 1 {
            let path = self.frame()?.node.path.join(name);
            if !self.claims((md.dev(), md.ino()), &path) {
                node.own_allocated = 0;
                node.own_apparent = 0;
                node.flags |= flags::ALIAS;
            }
            node.path = path;
        }
        self.place(node, name, slot)
    }

    /// The first pass names the owner of every inode it saw. An inode linked after it
    /// went by has none yet: the first path that shows up keeps the bytes.
    fn claims(&mut self, key: (u64, u64), path: &Path) -> bool {
        match self.owners.get(&key) {
            Some(owner) => owner == path,
            None => {
                self.owners.insert(key, path.to_path_buf());
                true
            }
        }
    }

    fn step(&mut self) -> Result<()> {
        let next = self.frame()?.iter.next();
        match next {
            Some(Ok(entry)) => self.entry(&entry),
            Some(Err(_)) => {
                // readdir failed mid-stream: the listing is incomplete
                let frame = self.frame()?;
                frame.node.flags |= flags::DENIED;
                frame.node.own_errors += 1;
                frame.node.total.errors += 1;
                self.finish()
            }
            None => self.finish(),
        }
    }

    /// Whether a finished directory stays in the snapshot: it is large, it is busy, or
    /// something below it must stay.
    fn keeps(&self, frame: &Frame) -> bool {
        large(&frame.node.total, self.w.threshold) || frame.kept_child || frame.node.pinned()
    }

    fn finish(&mut self) -> Result<()> {
        let mut frame = self
            .stack
            .pop()
            .ok_or_else(|| invariant("walk stack is empty"))?;
        if self.stack.is_empty() {
            self.top = Some(Top {
                node: frame.node,
                slots: frame.slots,
            });
            return Ok(());
        }
        let keep = self.keeps(&frame);
        let parent = self.frame()?;
        parent.node.total.add(&frame.node.total);
        if !keep {
            parent
                .slots
                .entry(frame.slot)
                .or_default()
                .add(&frame.node.total);
            return Ok(());
        }
        parent.kept_child = true;
        frame.node.buckets = tile(&frame.slots, BUCKET_ENTRIES, 0, SLOTS)?;
        self.retain(frame.node)
    }

    /// Start the walk at `top`. A directory that cannot be listed ends the walk at once.
    fn open_top(&mut self, mut node: Node, path: &Path) {
        node.path = path.to_path_buf();
        match fs::read_dir(path) {
            Ok(iter) => {
                node.total = node.own();
                self.stack.push(Frame {
                    node,
                    iter,
                    slot: 0,
                    slots: Slots::new(),
                    kept_child: false,
                });
            }
            Err(_) => {
                node.flags |= flags::DENIED;
                node.own_errors += 1;
                node.total = node.own();
                self.top = Some(Top {
                    node,
                    slots: Slots::new(),
                });
            }
        }
    }

    fn into_walked(mut self) -> Result<Walked> {
        let top = self
            .top
            .take()
            .ok_or_else(|| invariant("walk produced no top directory"))?;
        let mut walked = Walked {
            visited: self.visited,
            ..Walked::default()
        };
        match self.w.scope {
            None => {
                let mut node = top.node;
                node.buckets = tile(&top.slots, BUCKET_ENTRIES, 0, SLOTS)?;
                self.retained.push(node);
            }
            Some(scope) => {
                walked.unreadable = top.node.flags & flags::DENIED != 0;
                walked.buckets = tile(&top.slots, BUCKET_ENTRIES, scope.lo, scope.hi)?;
                walked.parent = Some(top.node);
            }
        }
        walked.nodes = self.retained;
        Ok(walked)
    }
}

/// Second pass over `top`: fold what is small, keep the rest. `root` marks `top` as a
/// scan root of the snapshot.
pub fn fold(top: &Path, root: bool, w: &Walk<'_>, owners: &mut Owners) -> Result<Walked> {
    let meta =
        fs::symlink_metadata(top).map_err(|e| Error::io(format!("stat {}", top.display()), e))?;
    if !meta.is_dir() {
        return Err(Error::usage(format!(
            "{} is not a directory",
            top.display()
        )));
    }
    let mut node = described(Kind::Dir, &meta, platform::now_secs());
    node.root = root;
    let mut fold = Fold {
        w,
        owners,
        stack: Vec::new(),
        retained: Vec::new(),
        top: None,
        visited: 0,
    };
    fold.open_top(node, top);
    while !fold.stack.is_empty() {
        fold.step()?;
    }
    fold.into_walked()
}

/// Both passes over `top` with a private owner table. The flag says whether the walked
/// tree held any hardlinked inode: their ownership depends on paths outside a region, so
/// callers refreshing a part of a snapshot must not trust such a result.
pub fn scan(top: &Path, root: bool, w: &Walk<'_>) -> Result<(Walked, bool)> {
    let mut owners = Owners::new();
    link_owners(top, w, &mut owners);
    // progress counts the second pass alone, which is the one that ends in a result
    w.seen.store(0, Ordering::Relaxed);
    let walked = fold(top, root, w, &mut owners)?;
    Ok((walked, !owners.is_empty()))
}

#[cfg(test)]
mod tests;

//! What survives of a scanned tree in a coarse snapshot: the roots, every entry that is
//! large enough (or must stay for another reason), and per directory a set of buckets
//! that hold the totals of everything folded away below it.

use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use crate::config::PPM;
use crate::error::{Error, ErrorCode, Result};
use crate::platform::FsStats;
use crate::scan::{Kind, flags};

/// Hash slots per retained directory. Folded children are spread over `[0, SLOTS)` by name.
pub const SLOT_BITS: u32 = 12;
pub const SLOTS: u32 = 1 << SLOT_BITS;
/// Largest number of entries a bucket may hold unless one slot alone exceeds it, and the
/// entry count above which a directory is retained whatever its allocated size.
pub const BUCKET_ENTRIES: u64 = 4096;
/// Retained entries a snapshot may hold. A threshold that keeps more is a mistake: the
/// result would be as large as a v1 snapshot without being one.
pub const MAX_NODES: usize = 200_000;
/// How many rows a first coarse snapshot is assumed to need when its size must be planned
/// before anything was scanned (`doctor`): far above what the measured snapshots hold.
pub const FIRST_SCAN_ROWS: u64 = 250_000;
/// Directories nested deeper than this are not entered (they count as unreadable): every
/// level of the walk keeps a directory handle open.
pub const MAX_DEPTH: usize = 512;
/// Version of the side tables next to the v1 tables.
pub const FORMAT: i64 = 1;

/// Totals of a set of entries. Used for retained subtrees, buckets and single entries.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub allocated: u64,
    pub apparent: u64,
    /// Files, directories, symlinks and the rest together: the unit of refresh work.
    pub entries: u64,
    pub errors: u64,
    /// Non-directory entries whose inode has several links, owners and aliases alike.
    pub links: u64,
    pub min_scanned_at: Option<i64>,
    pub newest_mtime: Option<i64>,
}

impl Stats {
    pub fn add(&mut self, other: &Self) {
        self.allocated = self.allocated.saturating_add(other.allocated);
        self.apparent = self.apparent.saturating_add(other.apparent);
        self.entries = self.entries.saturating_add(other.entries);
        self.errors = self.errors.saturating_add(other.errors);
        self.links = self.links.saturating_add(other.links);
        self.min_scanned_at = pick(self.min_scanned_at, other.min_scanned_at, i64::min);
        self.newest_mtime = pick(self.newest_mtime, other.newest_mtime, i64::max);
    }
}

fn pick(a: Option<i64>, b: Option<i64>, choose: fn(i64, i64) -> i64) -> Option<i64> {
    match (a, b) {
        (Some(x), Some(y)) => Some(choose(x, y)),
        (x, None) | (None, x) => x,
    }
}

/// Folded children whose name hash falls in `[lo, hi)`. The buckets of a retained
/// directory tile `[0, SLOTS)`, so a name always maps to exactly one of them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Bucket {
    pub lo: u32,
    pub hi: u32,
    pub stats: Stats,
}

/// What `mounts` knows about a mount point that the scan did not enter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MountMeta {
    pub fs_type: String,
    pub stats: Option<FsStats>,
}

#[derive(Clone, Debug)]
pub struct Node {
    pub path: PathBuf,
    /// A scan root has no parent in the snapshot.
    pub root: bool,
    pub kind: Kind,
    pub flags: u8,
    pub nlink: u32,
    pub mtime: i64,
    pub scanned_at: i64,
    /// Bytes of this entry alone: its blocks and length, or 0 for a hardlink alias.
    pub own_allocated: u64,
    pub own_apparent: u64,
    pub own_errors: u64,
    /// Own values plus buckets plus retained children; kept in step by `merge::rebuild`.
    pub total: Stats,
    /// Directories only: the tiling of `[0, SLOTS)`.
    pub buckets: Vec<Bucket>,
    pub mount: Option<Box<MountMeta>>,
}

impl Node {
    pub fn is_dir(&self) -> bool {
        self.kind == Kind::Dir
    }

    pub fn parent(&self) -> Option<&Path> {
        if self.root { None } else { self.path.parent() }
    }

    pub fn name(&self) -> Option<&[u8]> {
        self.path.file_name().map(OsStrExt::as_bytes)
    }

    /// An entry that is retained whatever its size: its absence from the snapshot would
    /// hide a mount point or an unreadable directory.
    pub fn pinned(&self) -> bool {
        self.kind == Kind::Mount || self.flags & flags::DENIED != 0
    }

    /// The entry alone, as it adds to its parent's totals.
    pub fn own(&self) -> Stats {
        let linked = self.nlink > 1 && !matches!(self.kind, Kind::Dir | Kind::Mount);
        Stats {
            allocated: self.own_allocated,
            apparent: self.own_apparent,
            entries: 1,
            errors: self.own_errors,
            links: u64::from(linked),
            min_scanned_at: Some(self.scanned_at),
            newest_mtime: (self.kind == Kind::File).then(|| self.mtime.min(self.scanned_at)),
        }
    }

    /// Everything below this node that was folded away: the sum of its buckets.
    pub fn rest(&self) -> Stats {
        let mut sum = Stats::default();
        for bucket in &self.buckets {
            sum.add(&bucket.stats);
        }
        sum
    }
}

/// One scan root and what was known about its filesystem when it was scanned.
#[derive(Clone, Debug)]
pub struct RootMeta {
    pub path: PathBuf,
    pub fs_type: String,
    pub stats: FsStats,
    /// Entries (and directory subtrees) below this many allocated bytes are folded.
    pub threshold: u64,
}

#[derive(Clone, Debug, Default)]
pub struct Model {
    pub roots: Vec<RootMeta>,
    pub nodes: Vec<Node>,
    /// The folding threshold as it was configured, in parts per million of a root's size.
    pub ppm: u32,
}

impl Model {
    /// Entries the scan found on disk, retained or folded.
    pub fn entries(&self) -> u64 {
        self.nodes
            .iter()
            .filter(|n| n.root)
            .map(|n| n.total.entries)
            .sum()
    }

    /// Entries the scan could not read or stat.
    pub fn errors(&self) -> u64 {
        self.nodes
            .iter()
            .filter(|n| n.root)
            .map(|n| n.total.errors)
            .sum()
    }

    /// The root whose tree holds `path`, the deepest one if roots are nested.
    pub fn root_of(&self, path: &Path) -> Option<&RootMeta> {
        self.roots
            .iter()
            .filter(|r| path.starts_with(&r.path))
            .max_by_key(|r| r.path.as_os_str().len())
    }
}

/// Smallest entry (in allocated bytes) a root keeps: `ppm` parts per million of its size.
pub fn threshold(capacity: u64, ppm: u32) -> Result<u64> {
    if capacity == 0 {
        return Err(Error::generic(
            "the filesystem reports no size, so the coarse backend cannot choose a folding threshold",
        ));
    }
    let bytes = u128::from(capacity) * u128::from(ppm);
    Ok(u64::try_from(bytes.div_ceil(u128::from(PPM)))
        .unwrap_or(u64::MAX)
        .max(1))
}

/// A directory is retained for its size, or because folding it would make a refresh of
/// anything below it as expensive as rescanning everything below it.
pub fn large(total: &Stats, threshold: u64) -> bool {
    total.allocated >= threshold || total.entries > BUCKET_ENTRIES
}

/// The error for a snapshot whose stored data cannot be read back as a coarse snapshot.
pub fn unreadable(what: &str) -> Error {
    Error::new(
        ErrorCode::Schema,
        format!("not a usable coarse snapshot: {what}"),
    )
    .with_fix("run a full scan: sudo freespace scan")
}

/// A broken internal invariant or a snapshot that does not satisfy one.
pub fn invariant(what: &str) -> Error {
    Error::generic(format!("coarse snapshot invariant violated: {what}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn threshold_rounds_up_and_never_reaches_zero() {
        assert_eq!(threshold(1_000_000, 1000).unwrap(), 1000);
        assert_eq!(threshold(1_000_001, 1000).unwrap(), 1001);
        assert_eq!(threshold(10, 1).unwrap(), 1);
        assert_eq!(threshold(u64::MAX, 1_000_000).unwrap(), u64::MAX);
        assert!(threshold(0, 1000).is_err());
    }

    #[test]
    fn stats_add_keeps_extremes_and_saturates() {
        let mut a = Stats {
            allocated: u64::MAX,
            entries: 1,
            min_scanned_at: Some(50),
            newest_mtime: None,
            ..Stats::default()
        };
        let b = Stats {
            allocated: 5,
            entries: 2,
            min_scanned_at: Some(40),
            newest_mtime: Some(7),
            ..Stats::default()
        };
        a.add(&b);
        assert_eq!((a.allocated, a.entries), (u64::MAX, 3));
        assert_eq!((a.min_scanned_at, a.newest_mtime), (Some(40), Some(7)));
    }

    #[test]
    fn own_counts_links_and_newest_only_for_the_right_kinds() {
        let mut n = Node {
            path: "/a/b".into(),
            root: false,
            kind: Kind::File,
            flags: 0,
            nlink: 2,
            mtime: 900,
            scanned_at: 800,
            own_allocated: 4096,
            own_apparent: 10,
            own_errors: 0,
            total: Stats::default(),
            buckets: Vec::new(),
            mount: None,
        };
        let own = n.own();
        assert_eq!(
            (own.links, own.newest_mtime, own.entries),
            (1, Some(800), 1)
        );
        n.kind = Kind::Dir;
        let own = n.own();
        assert_eq!((own.links, own.newest_mtime), (0, None));
        assert_eq!(n.parent(), Some(Path::new("/a")));
        n.root = true;
        assert_eq!(n.parent(), None);
        assert_eq!(n.name(), Some(&b"b"[..]));
    }
}

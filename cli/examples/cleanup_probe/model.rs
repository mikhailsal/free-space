use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

pub const MAX_RECORDS: usize = 100_000;
pub const MAX_BUCKETS: usize = 400_000;
pub const MAX_DEPTH: usize = 256;
/// Hash slots per retained directory. Omitted children are spread over `[0, SLOTS)` by name.
pub const SLOT_BITS: u32 = 12;
pub const SLOTS: u32 = 1 << SLOT_BITS;
/// Largest number of entries a bucket may hold unless one slot alone exceeds it, and the
/// entry count above which a directory is retained whatever its allocated size.
pub const BUCKET_ENTRIES: u64 = 4096;
/// Snapshot representation: 2 = coarse nodes plus bucketed residuals.
pub const FORMAT: i64 = 2;
pub type R<T> = Result<T, Box<dyn std::error::Error>>;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub allocated: u64,
    pub apparent: u64,
    pub files: u64,
    pub dirs: u64,
    pub others: u64,
    pub errors: u64,
    /// Non-directory entries whose inode has several links, owners and aliases alike.
    pub links: u64,
    pub min_scanned_at: Option<i64>,
    pub newest_mtime: Option<i64>,
}
impl Stats {
    pub fn add(&mut self, other: &Self) -> R<()> {
        macro_rules! add { ($($x:ident),+) => { $(self.$x = self.$x.checked_add(other.$x).ok_or("aggregate overflow")?;)+ }; }
        add!(allocated, apparent, files, dirs, others, errors, links);
        self.min_scanned_at = minimum(self.min_scanned_at, other.min_scanned_at);
        self.newest_mtime = maximum(self.newest_mtime, other.newest_mtime);
        Ok(())
    }
    /// Files, directories and other entries together; this is the unit of refresh work.
    pub fn entries(&self) -> u64 {
        self.files
            .saturating_add(self.dirs)
            .saturating_add(self.others)
    }
}
pub fn minimum(a: Option<i64>, b: Option<i64>) -> Option<i64> {
    match (a, b) {
        (Some(x), Some(y)) => Some(x.min(y)),
        (x, None) | (None, x) => x,
    }
}
pub fn maximum(a: Option<i64>, b: Option<i64>) -> Option<i64> {
    match (a, b) {
        (Some(x), Some(y)) => Some(x.max(y)),
        (x, None) | (None, x) => x,
    }
}

/// Omitted children whose name hash falls in `[lo, hi)`. The buckets of a retained
/// directory tile `[0, SLOTS)`, so a path always maps to exactly one of them.
#[derive(Clone, Debug, Default)]
pub struct Bucket {
    pub lo: u32,
    pub hi: u32,
    pub stats: Stats,
}

#[derive(Clone)]
pub struct Node {
    pub path: PathBuf,
    pub parent: Option<PathBuf>,
    pub kind: u8,
    pub own_allocated: u64,
    pub own_apparent: u64,
    pub own_errors: u64,
    pub total: Stats,
    pub scanned_at: i64,
    pub buckets: Vec<Bucket>,
}
impl Node {
    pub fn own(&self) -> Stats {
        Stats {
            allocated: self.own_allocated,
            apparent: self.own_apparent,
            files: u64::from(self.kind == 0),
            dirs: u64::from(self.kind == 1),
            others: u64::from(self.kind == 2),
            errors: self.own_errors,
            links: if self.kind == 1 { 0 } else { self.total.links },
            min_scanned_at: Some(self.scanned_at),
            newest_mtime: if self.kind == 0 {
                self.total.newest_mtime
            } else {
                None
            },
        }
    }
    /// Everything below this node that was folded away: the sum of its buckets.
    pub fn rest(&self) -> R<Stats> {
        let mut sum = Stats::default();
        for bucket in &self.buckets {
            sum.add(&bucket.stats)?;
        }
        Ok(sum)
    }
    pub fn name(&self) -> Option<&[u8]> {
        self.path.file_name().map(OsStrExt::as_bytes)
    }
}
pub fn hex(path: &Path) -> String {
    path.as_os_str()
        .as_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
pub fn threshold(capacity: u64, ppm: u32) -> R<u64> {
    if capacity == 0 || ppm == 0 || ppm > 1_000_000 {
        return Err("capacity and ppm must be positive; ppm <= 1000000".into());
    }
    u64::try_from(((capacity as u128) * (ppm as u128)).div_ceil(1_000_000)).map_err(Into::into)
}

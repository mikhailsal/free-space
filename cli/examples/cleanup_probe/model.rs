use serde::Serialize;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

pub const MAX_RECORDS: usize = 100_000;
pub const MAX_DEPTH: usize = 256;
pub type R<T> = Result<T, Box<dyn std::error::Error>>;

#[derive(Clone, Default, Serialize)]
pub struct Stats {
    pub allocated: u64,
    pub apparent: u64,
    pub files: u64,
    pub dirs: u64,
    pub others: u64,
    pub errors: u64,
    pub min_scanned_at: Option<i64>,
    pub newest_mtime: Option<i64>,
}
impl Stats {
    pub fn add(&mut self, other: &Self) -> R<()> {
        macro_rules! add { ($($x:ident),+) => { $(self.$x = self.$x.checked_add(other.$x).ok_or("aggregate overflow")?;)+ }; }
        add!(allocated, apparent, files, dirs, others, errors);
        self.min_scanned_at = minimum(self.min_scanned_at, other.min_scanned_at);
        self.newest_mtime = maximum(self.newest_mtime, other.newest_mtime);
        Ok(())
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

#[derive(Clone)]
pub struct Node {
    pub path: PathBuf,
    pub parent: Option<PathBuf>,
    pub kind: u8,
    pub own_allocated: u64,
    pub own_apparent: u64,
    pub own_errors: u64,
    pub total: Stats,
    pub rest: Stats,
    pub scanned_at: i64,
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
            min_scanned_at: Some(self.scanned_at),
            newest_mtime: if self.kind == 0 {
                self.total.newest_mtime
            } else {
                None
            },
        }
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
    u64::try_from(((capacity as u128) * (ppm as u128) + 999_999) / 1_000_000).map_err(Into::into)
}

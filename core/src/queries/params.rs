//! Query parameters shared by the CLI and the HTTP query string (T8): the CLI
//! fills these structs from clap, `/api/*` deserializes them from the URL, and
//! the same validation runs for both.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::format;

pub const DEFAULT_LIMIT: u32 = 30;
pub const MAX_LIMIT: u32 = 10_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "lowercase")]
pub enum SortKey {
    #[default]
    Allocated,
    Apparent,
    Mtime,
}

impl SortKey {
    pub fn as_str(self) -> &'static str {
        match self {
            SortKey::Allocated => "allocated",
            SortKey::Apparent => "apparent",
            SortKey::Mtime => "mtime",
        }
    }

    pub fn parse(s: &str) -> Result<SortKey> {
        match s {
            "allocated" => Ok(SortKey::Allocated),
            "apparent" => Ok(SortKey::Apparent),
            "mtime" => Ok(SortKey::Mtime),
            _ => Err(Error::usage(format!(
                "bad sort {s:?}: use allocated, apparent or mtime"
            ))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum KindFilter {
    File,
    Dir,
    Any,
}

impl KindFilter {
    pub fn as_str(self) -> &'static str {
        match self {
            KindFilter::File => "file",
            KindFilter::Dir => "dir",
            KindFilter::Any => "any",
        }
    }

    pub fn parse(s: &str) -> Result<KindFilter> {
        match s {
            "file" => Ok(KindFilter::File),
            "dir" => Ok(KindFilter::Dir),
            "any" => Ok(KindFilter::Any),
            _ => Err(Error::usage(format!(
                "bad kind {s:?}: use file, dir or any"
            ))),
        }
    }
}

/// `path` (UTF-8) or `path_raw` (hex of the exact bytes); default "/".
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PathArg {
    pub path: Option<String>,
    pub path_raw: Option<String>,
}

impl PathArg {
    pub fn new(path: Option<String>, path_raw: Option<String>) -> Self {
        PathArg { path, path_raw }
    }

    pub fn bytes(&self) -> Result<Vec<u8>> {
        match (&self.path, &self.path_raw) {
            (Some(_), Some(_)) => Err(Error::usage("give either a path or --path-raw, not both")),
            (_, Some(h)) => hex_decode(h),
            (Some(p), None) => Ok(p.as_bytes().to_vec()),
            (None, None) => Ok(b"/".to_vec()),
        }
    }
}

pub fn hex_encode(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

pub fn hex_decode(h: &str) -> Result<Vec<u8>> {
    let h = h.trim();
    if !h.len().is_multiple_of(2) || !h.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err(Error::usage(format!(
            "bad --path-raw {h:?}: expected hex bytes"
        )));
    }
    Ok((0..h.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&h[i..i + 2], 16).unwrap_or(0))
        .collect())
}

pub fn check_limit(limit: Option<u32>) -> Result<u32> {
    match limit {
        None => Ok(DEFAULT_LIMIT),
        Some(0) => Err(Error::usage("limit must be at least 1")),
        Some(n) if n > MAX_LIMIT => Err(Error::usage(format!(
            "limit {n} is above the maximum {MAX_LIMIT}"
        ))),
        Some(n) => Ok(n),
    }
}

/// `--min-size`: non-negative integer with optional K|M|G|T (binary, any case).
pub fn parse_size(s: &str) -> Result<u64> {
    let t = s.trim();
    let bad = || {
        Error::usage(format!(
            "bad size {s:?}: use a number with an optional K, M, G or T suffix, e.g. 100M"
        ))
    };
    let (num, mul) = match t.chars().last() {
        Some(c) if c.is_ascii_alphabetic() => {
            let m: u64 = match c.to_ascii_uppercase() {
                'K' => 1 << 10,
                'M' => 1 << 20,
                'G' => 1 << 30,
                'T' => 1 << 40,
                _ => return Err(bad()),
            };
            (&t[..t.len() - 1], m)
        }
        _ => (t, 1),
    };
    if num.is_empty() || !num.bytes().all(|c| c.is_ascii_digit()) {
        return Err(bad());
    }
    num.parse::<u64>()
        .ok()
        .and_then(|n| n.checked_mul(mul))
        .ok_or_else(bad)
}

/// `--older-than` / `--newer-than`: `YYYY-MM-DD` (local midnight) or `Nd|Nw|Nmo|Ny`
/// (mo = 30 days, y = 365 days) before `now`. Returns the cut-off timestamp.
pub fn parse_time_bound(s: &str, now: i64) -> Result<i64> {
    let t = s.trim();
    let bad = || {
        Error::usage(format!(
            "bad date {s:?}: use YYYY-MM-DD or a duration like 180d, 4w, 6mo, 1y"
        ))
    };
    if t.len() == 10 && t.as_bytes()[4] == b'-' && t.as_bytes()[7] == b'-' {
        let y: i32 = t[0..4].parse().map_err(|_| bad())?;
        let m: u32 = t[5..7].parse().map_err(|_| bad())?;
        let d: u32 = t[8..10].parse().map_err(|_| bad())?;
        return format::local_midnight(y, m, d).ok_or_else(bad);
    }
    let split = t.find(|c: char| !c.is_ascii_digit()).ok_or_else(bad)?;
    let (n, unit) = t.split_at(split);
    let n: i64 = n.parse().map_err(|_| bad())?;
    let day = 86_400i64;
    let secs = match unit {
        "d" => day,
        "w" => 7 * day,
        "mo" => 30 * day,
        "y" => 365 * day,
        _ => return Err(bad()),
    };
    n.checked_mul(secs).map(|d| now - d).ok_or_else(bad)
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LsParams {
    pub path: Option<String>,
    pub path_raw: Option<String>,
    pub sort: Option<String>,
    pub limit: Option<u32>,
    /// `snapshot_id` the caller is looking at; a different current snapshot → ROTATED.
    pub snapshot: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TopParams {
    pub path: Option<String>,
    pub path_raw: Option<String>,
    pub kind: Option<String>,
    pub min_size: Option<String>,
    pub older_than: Option<String>,
    pub sort: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FindParams {
    pub path: Option<String>,
    pub path_raw: Option<String>,
    pub name: Option<String>,
    pub iname: Option<String>,
    pub older_than: Option<String>,
    pub newer_than: Option<String>,
    pub min_size: Option<String>,
    pub kind: Option<String>,
    pub sort: Option<String>,
    pub limit: Option<u32>,
    pub snapshot: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffParams {
    pub path: Option<String>,
    pub path_raw: Option<String>,
    pub depth: Option<u32>,
    pub kind: Option<String>,
    pub limit: Option<u32>,
}

pub const MAX_DEPTH: u32 = 64;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes() {
        assert_eq!(parse_size("0").unwrap(), 0);
        assert_eq!(parse_size("100M").unwrap(), 100 << 20);
        assert_eq!(parse_size("1g").unwrap(), 1 << 30);
        assert_eq!(parse_size("2T").unwrap(), 2 << 40);
        assert_eq!(parse_size("512").unwrap(), 512);
        for bad in ["", "M", "-1", "1.5G", "10X", "1MB"] {
            assert!(parse_size(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn durations_and_dates() {
        let now = 1_000_000_000;
        assert_eq!(parse_time_bound("180d", now).unwrap(), now - 180 * 86400);
        assert_eq!(parse_time_bound("2w", now).unwrap(), now - 14 * 86400);
        assert_eq!(parse_time_bound("6mo", now).unwrap(), now - 180 * 86400);
        assert_eq!(parse_time_bound("1y", now).unwrap(), now - 365 * 86400);
        let d = parse_time_bound("2023-01-01", now).unwrap();
        assert_eq!(format::local_date(d), "2023-01-01");
        for bad in ["", "d", "10", "10m", "2023-13-01", "2023/01/01", "-5d"] {
            assert!(parse_time_bound(bad, now).is_err(), "{bad}");
        }
    }

    #[test]
    fn limits() {
        assert_eq!(check_limit(None).unwrap(), 30);
        assert!(check_limit(Some(0)).is_err());
        assert!(check_limit(Some(99_999_999)).is_err());
        assert_eq!(check_limit(Some(10_000)).unwrap(), 10_000);
    }

    #[test]
    fn path_arg() {
        assert_eq!(PathArg::default().bytes().unwrap(), b"/");
        assert_eq!(
            PathArg::new(None, Some("2f61ff".into())).bytes().unwrap(),
            b"/a\xff"
        );
        assert!(
            PathArg::new(Some("/".into()), Some("2f".into()))
                .bytes()
                .is_err()
        );
        assert!(PathArg::new(None, Some("zz".into())).bytes().is_err());
        assert_eq!(hex_encode(b"/a\xff"), "2f61ff");
    }
}

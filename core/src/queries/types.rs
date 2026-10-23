//! JSON response types. Every struct derives `JsonSchema`; `freespace schema
//! --json` publishes them as `skill/schema.json` (T22).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::scan::Kind;
use crate::snapshot::lock::Scanning;
use crate::snapshot::progress::Progress;
use crate::snapshot::state::LastError;

/// Fields every snapshot answer carries.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Header {
    pub host: String,
    pub os: String,
    pub generation: u64,
    pub prev_generation: Option<u64>,
    /// Start of the last full scan (UNIX seconds).
    pub scanned_at: i64,
    /// Age of the oldest part of the requested subtree.
    pub age_seconds: i64,
    pub schema_version: i64,
    pub snapshot_id: String,
    /// Some directories could not be read: sizes under `unknown_subtrees` are unknown (D13).
    /// Report this to the user before any numbers.
    pub incomplete: bool,
    pub unknown_subtrees: Vec<String>,
}

/// One snapshot entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct EntryOut {
    /// Full path, UTF-8 (invalid bytes replaced).
    pub path: String,
    /// Hex of the exact path bytes; present only when `path` is lossy. Pass it
    /// back with `--path-raw` / `?path_raw=`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path_raw: Option<String>,
    pub name: String,
    pub kind: Kind,
    /// Bytes on disk (st_blocks × 512), subtree total for directories.
    pub allocated: u64,
    /// Apparent size (st_size), subtree total for directories.
    pub apparent: u64,
    /// UNIX seconds; 0 = unknown.
    pub mtime: i64,
    /// Newest file inside (directories) or `min(mtime, scanned_at)` (files); 0 = unknown.
    pub newest_mtime: i64,
    pub nlink: u64,
    /// Descendant count (directories).
    pub items: u64,
    /// Unreadable entries in the subtree.
    pub errors: u64,
    pub scanned_at: i64,
    /// This directory could not be read.
    #[serde(default, skip_serializing_if = "is_false")]
    pub denied: bool,
    /// Hardlink counted under another path (`allocated` is 0 here).
    #[serde(default, skip_serializing_if = "is_false")]
    pub hardlink_alias: bool,
    /// For `nlink > 1`: the path that carries the size.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hardlink_owner: Option<String>,
    /// Filesystem mounted here (kind = mount).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mount: Option<MountOut>,
}

fn is_false(b: &bool) -> bool {
    !*b
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MountOut {
    pub fs_type: String,
    pub used: Option<u64>,
    pub total: Option<u64>,
    /// The scanned root for this mount point, when it is in the root set.
    pub root: Option<String>,
}

/// `ls`: the children of one folder.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LsResponse {
    #[serde(flatten)]
    pub header: Header,
    pub entry: EntryOut,
    pub entries: Vec<EntryOut>,
    /// Number of children of `entry`.
    pub total_count: u64,
    pub truncated: bool,
    pub limit: u32,
    pub sort: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
pub struct Total {
    /// All matches.
    pub count: u64,
    /// Matches that add to the sums (not nested in another match).
    pub counted: u64,
    pub allocated: u64,
    pub apparent: u64,
}

/// `top`: the largest files, or the topmost folders matching the filters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TopResponse {
    #[serde(flatten)]
    pub header: Header,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path_raw: Option<String>,
    /// "file" or "dir"
    pub kind: String,
    pub min_size: Option<u64>,
    /// Cut-off timestamp of `--older-than`.
    pub older_than: Option<i64>,
    pub sort: String,
    pub entries: Vec<EntryOut>,
    pub total: Total,
    pub total_count: u64,
    pub total_allocated: u64,
    pub truncated: bool,
    pub limit: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FindEntry {
    #[serde(flatten)]
    pub entry: EntryOut,
    /// Inside another matched directory: counted in `total.count` only.
    pub nested: bool,
}

/// `find`: matches under a path, with totals over all of them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FindResponse {
    #[serde(flatten)]
    pub header: Header,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path_raw: Option<String>,
    pub kind: String,
    pub sort: String,
    pub entries: Vec<FindEntry>,
    /// Over all matches, not just `entries`.
    pub total: Total,
    pub total_count: u64,
    pub total_allocated: u64,
    pub truncated: bool,
    pub limit: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum DiffStatus {
    New,
    Gone,
    Changed,
    Same,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DiffEntry {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path_raw: Option<String>,
    pub name: String,
    pub kind: Kind,
    pub status: DiffStatus,
    pub allocated: u64,
    pub allocated_prev: u64,
    pub delta: i64,
    pub items: u64,
    pub items_prev: u64,
    pub mtime: i64,
    pub newest_mtime: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Refreshed {
    pub path: String,
    pub scanned_at: i64,
    pub generation: u64,
}

/// `diff`: what changed since the previous full scan.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DiffResponse {
    #[serde(flatten)]
    pub header: Header,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path_raw: Option<String>,
    pub prev_scanned_at: i64,
    pub depth: u32,
    pub kind: String,
    /// Subtrees were refreshed after `prev`: deltas mix two baselines.
    pub partial: bool,
    pub refreshed: Vec<Refreshed>,
    /// The root set differs between the generations.
    pub roots_changed: bool,
    /// allocated(current) − allocated(prev) of the requested path itself.
    pub delta: i64,
    pub entries: Vec<DiffEntry>,
    pub total_count: u64,
    pub truncated: bool,
    pub limit: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RootOut {
    pub path: String,
    pub fs_type: String,
    /// Bytes counted by the scan under this root.
    pub allocated: u64,
    pub items: u64,
    /// statvfs at scan time.
    pub fs_used: u64,
    pub fs_total: u64,
    pub fs_avail: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FsOut {
    pub path: String,
    pub fs_type: String,
    pub total: u64,
    pub used: u64,
    /// Available now (live statvfs); null when the mount is gone.
    pub avail: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PrevOut {
    pub generation: u64,
    pub scanned_at: i64,
}

/// `status`: snapshot identity and freshness, filesystems, last error, running scan.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct StatusResponse {
    pub host: String,
    pub os: String,
    pub version: String,
    pub snapshot_dir: String,
    /// null until the first snapshot is published.
    pub generation: Option<u64>,
    pub prev_generation: Option<u64>,
    pub schema_version: Option<i64>,
    pub snapshot_id: Option<String>,
    pub scanned_at: Option<i64>,
    pub age_seconds: Option<i64>,
    pub entries: Option<u64>,
    pub prev: Option<PrevOut>,
    pub refreshed: Vec<Refreshed>,
    pub roots: Vec<RootOut>,
    pub fs: Vec<FsOut>,
    pub incomplete: bool,
    pub unknown_subtrees: Vec<String>,
    /// From state.json.
    pub last_error: Option<LastError>,
    pub last_success_ts: Option<i64>,
    pub warnings: Vec<String>,
    /// state.json exists but cannot be read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state_error: Option<String>,
    /// A scan or refresh holds the lock right now.
    pub scanning: Option<Scanning>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<Progress>,
}

/// Every answer type, for `freespace schema --json`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum AnyResponse {
    Status(Box<StatusResponse>),
    Ls(Box<LsResponse>),
    Top(Box<TopResponse>),
    Find(Box<FindResponse>),
    Diff(Box<DiffResponse>),
    Error(Box<crate::error::ErrorBody>),
}

/// The document `freespace schema --json` prints and `skill/schema.json` holds:
/// every response under `$defs`, plus a `commands` map from command to its answer.
pub fn schema_document() -> serde_json::Value {
    let mut v = serde_json::to_value(schemars::schema_for!(AnyResponse)).unwrap_or_default();
    let commands = serde_json::json!({
        "status": {"$ref": "#/$defs/StatusResponse"},
        "ls": {"$ref": "#/$defs/LsResponse"},
        "top": {"$ref": "#/$defs/TopResponse"},
        "find": {"$ref": "#/$defs/FindResponse"},
        "diff": {"$ref": "#/$defs/DiffResponse"},
        "error": {"$ref": "#/$defs/ErrorBody"}
    });
    if let Some(o) = v.as_object_mut() {
        o.insert("title".into(), "freespace JSON answers".into());
        o.insert(
            "x-schema-version".into(),
            crate::snapshot::schema::SCHEMA_VERSION.into(),
        );
        o.insert("commands".into(), commands);
    }
    v
}

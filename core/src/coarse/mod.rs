//! The `coarse` backend. A coarse snapshot keeps the entries that are large enough to
//! matter (by default 0.1 % of the filesystem) and folds everything else into per-folder
//! `[small items]` rows that carry the folded totals. It is an ordinary v1 snapshot
//! file with three extra tables, so every query, `diff` and the GUI work on it unchanged;
//! only the way it is written and refreshed differs. See `docs/coarse-snapshot.md`.

mod buckets;
mod export;
mod import;
mod job;
mod merge;
mod model;
mod plan;
mod roots;
mod store;
#[cfg(test)]
mod testing;
mod walk;

pub use job::{
    Part, RefreshOutcome, RefreshRequest, Rescanned, ScanOutcome, ScanRequest, run_refresh,
    run_scan,
};
pub use model::FIRST_SCAN_ROWS;
pub use store::folded_below;

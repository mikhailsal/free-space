//! `refresh <path>`: rescan one folder and republish the current snapshot with
//! that subtree replaced (prev untouched, `refreshes` row appended).

use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;

use clap::Args;
use freespace_core::Error;
use freespace_core::config;
use freespace_core::format;
use freespace_core::job;
use freespace_core::refresh::{self, RefreshRequest};
use freespace_core::scan::default_workers;
use serde::Serialize;

use crate::output;
use crate::scan::{count, progress_line};

#[derive(Args)]
pub struct RefreshArgs {
    /// Folder to rescan; relative paths start at the current directory
    pub path: PathBuf,
    /// Walker threads (default: config `workers`, else min(CPUs, 8))
    #[arg(long)]
    pub workers: Option<usize>,
    /// Print the refresh report as JSON on stdout
    #[arg(long)]
    pub json: bool,
}

#[derive(Serialize)]
struct Report {
    generation: u64,
    path: String,
    /// Entries in the republished snapshot.
    entries: u64,
    /// Entries found under `path` by this walk.
    subtree_entries: u64,
    /// `path` no longer exists: its subtree was dropped.
    removed: bool,
    load_seconds: f64,
    walk_seconds: f64,
    finalize_seconds: f64,
    insert_seconds: f64,
    index_seconds: f64,
    publish_seconds: f64,
    total_seconds: f64,
    snapshot_bytes: u64,
    incomplete: bool,
    unknown_subtrees: Vec<String>,
}

pub fn run(a: RefreshArgs) -> output::Result<()> {
    let loaded = config::load()?;
    let workers = a
        .workers
        .or(loaded.config.workers)
        .unwrap_or_else(default_workers);
    if workers == 0 {
        return Err(Error::usage("--workers must be at least 1").into());
    }
    let path = if a.path.is_absolute() {
        a.path.clone()
    } else {
        let cwd = std::env::current_dir()
            .map_err(|e| Error::usage(format!("cannot resolve {}: {e}", a.path.display())))?;
        let joined = cwd.join(&a.path);
        std::fs::canonicalize(&joined).unwrap_or(joined)
    };
    let dir = loaded.snapshot_dir();
    let tty = output::stderr_is_tty();
    eprintln!(
        "freespace refresh {}: started, {workers} workers",
        path.display()
    );
    let rep = refresh::run_refresh(RefreshRequest {
        path: path.as_os_str().as_bytes().to_vec(),
        workers,
        dir,
        on_progress: progress_line(tty),
    });
    if tty {
        eprint!("\r\x1b[2K");
    }
    let rep = rep?;
    let what = if rep.removed {
        "gone from disk, dropped from the snapshot".to_owned()
    } else {
        format!("{} entries rescanned", count(rep.subtree_entries))
    };
    eprintln!(
        "freespace refresh {}: generation {} published, {what}, {} in {:.1} s",
        rep.path,
        rep.generation,
        format::size(rep.snapshot_bytes),
        rep.total_seconds
    );
    if let Some(w) = job::denied_warning(&rep.denied) {
        eprintln!("warning: {w}");
    }
    if a.json {
        output::print_json(&Report {
            generation: rep.generation,
            path: rep.path.clone(),
            entries: rep.entries,
            subtree_entries: rep.subtree_entries,
            removed: rep.removed,
            load_seconds: rep.load_seconds,
            walk_seconds: rep.walk_seconds,
            finalize_seconds: rep.finalize_seconds,
            insert_seconds: rep.write.insert_seconds,
            index_seconds: rep.write.index_seconds,
            publish_seconds: rep.publish_seconds,
            total_seconds: rep.total_seconds,
            snapshot_bytes: rep.snapshot_bytes,
            incomplete: !rep.denied.is_empty(),
            unknown_subtrees: rep.denied.clone(),
        })?;
    }
    Ok(())
}

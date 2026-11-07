//! `refresh <path>`: rescan one folder and republish the current snapshot with
//! that subtree replaced (prev untouched, `refreshes` row appended).

use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;

use clap::Args;
use freespace_core::coarse::{self, Part, Rescanned};
use freespace_core::config::{Backend, Loaded};
use freespace_core::format;
use freespace_core::job;
use freespace_core::refresh::{self, RefreshReport, RefreshRequest};
use freespace_core::scan::default_workers;
use freespace_core::snapshot::progress::ProgressCallback;
use freespace_core::{Error, Result};
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
    let loaded = crate::backend::load()?;
    let workers = a
        .workers
        .or(loaded.config.workers)
        .unwrap_or_else(default_workers);
    if workers == 0 {
        return Err(Error::usage("--workers must be at least 1").into());
    }
    let path = absolute(&a.path)?;
    if loaded.config.backend == Backend::V2 {
        crate::backend::timing_note();
    }
    let tty = output::stderr_is_tty();
    // the coarse walk is one thread, whatever --workers says
    let walking = if loaded.config.backend == Backend::Coarse {
        1
    } else {
        workers
    };
    eprintln!(
        "freespace refresh {}: started, {walking} workers",
        output::escape(&path.display().to_string())
    );
    let ran = execute(&loaded, &path, workers, progress_line(tty));
    if tty {
        eprint!("\r\x1b[2K");
    }
    let (rep, rescanned) = ran?;
    let what = if rep.removed {
        "gone from disk, dropped from the snapshot".to_owned()
    } else {
        format!("{} entries rescanned", count(rep.subtree_entries))
    };
    eprintln!(
        "freespace refresh {}: generation {} published, {what}, {} in {:.1} s",
        output::escape(&rep.path),
        rep.generation,
        format::size(rep.snapshot_bytes),
        rep.total_seconds
    );
    if let Some(note) = rescanned.and_then(|r| rescan_note(&path, &r)) {
        eprintln!("note: {}", output::escape(&note));
    }
    if let Some(w) = job::denied_warning(&rep.denied) {
        eprintln!("warning: {}", output::escape(&w));
    }
    if a.json {
        output::print_json(&report(&rep))?;
    }
    Ok(())
}

/// Relative paths start at the current directory, with symlinks resolved when it exists.
fn absolute(path: &std::path::Path) -> Result<PathBuf> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    let cwd = std::env::current_dir()
        .map_err(|e| Error::usage(format!("cannot resolve {}: {e}", path.display())))?;
    let joined = cwd.join(path);
    Ok(std::fs::canonicalize(&joined).unwrap_or(joined))
}

fn execute(
    loaded: &Loaded,
    path: &std::path::Path,
    workers: usize,
    on_progress: Option<ProgressCallback>,
) -> Result<(RefreshReport, Option<Rescanned>)> {
    let dir = crate::backend::dir(loaded);
    let bytes = path.as_os_str().as_bytes().to_vec();
    match loaded.config.backend {
        Backend::V2 => crate::backend::refresh(loaded, path, workers).map(|rep| (rep, None)),
        Backend::Coarse => coarse::run_refresh(coarse::RefreshRequest {
            path: bytes,
            dir,
            on_progress,
        })
        .map(|out| (out.report, Some(out.rescanned))),
        Backend::V1 => refresh::run_refresh(RefreshRequest {
            path: bytes,
            workers,
            dir,
            on_progress,
        })
        .map(|rep| (rep, None)),
    }
}

/// A coarse refresh rescans the part of the snapshot that holds the path, which can be more
/// than the path; say so when it is.
fn rescan_note(requested: &std::path::Path, r: &Rescanned) -> Option<String> {
    let at = r.path.display();
    match r.part {
        Part::Folder | Part::Root if r.path == requested => None,
        Part::Folder => Some(format!(
            "{at} was rescanned, the folder that lists the path"
        )),
        Part::Bucket => Some(format!(
            "the path is folded into the small items of {at}; that bucket was rescanned"
        )),
        Part::Root => Some(format!(
            "the whole root {at} was rescanned: the part to refresh holds hardlinks, whose sizes depend on other folders"
        )),
    }
}

fn report(rep: &RefreshReport) -> Report {
    Report {
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
    }
}

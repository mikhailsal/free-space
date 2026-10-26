use std::io::Write;
use std::path::PathBuf;

use clap::Args;
use freespace_core::Error;
use freespace_core::format;
use freespace_core::job::{self, ScanRequest};
use freespace_core::platform;
use freespace_core::scan::default_workers;
use freespace_core::snapshot::progress::ProgressCallback;
use serde::Serialize;

use crate::output;

#[derive(Args)]
pub struct ScanArgs {
    /// Roots to scan (default: `roots` from config.toml, else every real filesystem)
    pub roots: Vec<PathBuf>,
    /// Stay on each root's filesystem (default)
    #[arg(long, default_value_t = true, overrides_with = "cross_mounts")]
    pub one_file_system: bool,
    /// Descend into other filesystems mounted under a root
    #[arg(long)]
    pub cross_mounts: bool,
    /// Walker threads (default: config `workers`, else min(CPUs, 8))
    #[arg(long)]
    pub workers: Option<usize>,
    /// Stage entries on disk with one walker, trading speed and disk space for lower RAM
    #[arg(long)]
    pub low_memory: bool,
    /// Print the scan report as JSON on stdout
    #[arg(long)]
    pub json: bool,
}

#[derive(Serialize)]
struct Report<'a> {
    generation: u64,
    prev_generation: Option<u64>,
    entries: u64,
    roots: Vec<String>,
    workers: usize,
    walk_seconds: f64,
    finalize_seconds: f64,
    insert_seconds: f64,
    index_seconds: f64,
    publish_seconds: f64,
    total_seconds: f64,
    snapshot_bytes: u64,
    snapshot_path: String,
    errors: u64,
    incomplete: bool,
    unknown_subtrees: &'a freespace_core::scan::denied::DeniedPaths,
}

pub fn run(a: ScanArgs) -> output::Result<()> {
    let loaded = crate::backend::load()?;
    let roots = scan_roots(&a, &loaded);
    let workers = a
        .workers
        .or(loaded.config.workers)
        .unwrap_or_else(default_workers);
    if workers == 0 {
        return Err(Error::usage("--workers must be at least 1").into());
    }
    let low_memory = a.low_memory || loaded.config.low_memory;
    let workers = if low_memory { 1 } else { workers };
    let dir = crate::backend::dir(&loaded);
    if loaded.config.backend == freespace_core::config::Backend::V2 {
        crate::backend::timing_note();
    }
    let tty = output::stderr_is_tty();
    let label = roots_label(&roots);
    eprintln!(
        "freespace scan {label}: started, {workers} workers, snapshot dir {}",
        output::escape(&dir.path.display().to_string())
    );
    let on_progress = progress_line(tty);
    let rep = if loaded.config.backend == freespace_core::config::Backend::V2 {
        crate::backend::scan(&loaded, &roots, !a.cross_mounts, workers, low_memory)
    } else {
        job::run_scan(ScanRequest {
            roots: roots.clone(),
            one_file_system: !a.cross_mounts,
            workers,
            low_memory,
            dir: dir.clone(),
            keep_previous: loaded.config.keep_previous,
            on_progress,
        })
    };
    if tty {
        eprint!("\r\x1b[2K");
    }
    let rep = rep?;
    eprintln!(
        "freespace scan {label}: generation {} published, {} entries, {} in {:.1} s (walk {:.1} s, write {:.1} s)",
        rep.generation,
        count(rep.entries),
        format::size(rep.snapshot_bytes),
        rep.total_seconds,
        rep.walk_seconds,
        rep.write.insert_seconds + rep.write.index_seconds
    );
    if let Some(w) = rep.denied.warning() {
        eprintln!("warning: {}", output::escape(&w));
    }
    if a.json {
        output::print_json(&Report {
            generation: rep.generation,
            prev_generation: rep.prev_generation,
            entries: rep.entries,
            roots: roots.iter().map(|r| r.display().to_string()).collect(),
            workers,
            walk_seconds: rep.walk_seconds,
            finalize_seconds: rep.finalize_seconds,
            insert_seconds: rep.write.insert_seconds,
            index_seconds: rep.write.index_seconds,
            publish_seconds: rep.publish_seconds,
            total_seconds: rep.total_seconds,
            snapshot_bytes: rep.snapshot_bytes,
            snapshot_path: if loaded.config.backend == freespace_core::config::Backend::V2 {
                crate::backend::database(&dir)
            } else {
                dir.current()
            }
            .display()
            .to_string(),
            errors: rep.errors,
            incomplete: !rep.denied.is_empty(),
            unknown_subtrees: &rep.denied,
        })?;
    }
    Ok(())
}

/// The D14 progress line on a terminal: "walk: 2.1 M of ~7.3 M files · 29% · 48 s".
pub fn progress_line(tty: bool) -> Option<ProgressCallback> {
    if !tty {
        return None;
    }
    Some(Box::new(|p| {
        let expected = p
            .entries_expected
            .map(|e| format!(" of ~{}", count(e)))
            .unwrap_or_default();
        let pct = p.percent.map(|v| format!(" · {v}%")).unwrap_or_default();
        let mut err = std::io::stderr().lock();
        let _ = write!(
            err,
            "\r\x1b[2K{}: {}{expected} files{pct} · {} s",
            p.stage,
            count(p.entries_seen),
            p.elapsed_seconds
        );
        let _ = err.flush();
    }))
}

/// "2.1 M", "48 K", "512"
pub fn count(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1} M", n as f64 / 1e6)
    } else if n >= 10_000 {
        format!("{} K", n / 1000)
    } else {
        n.to_string()
    }
}

fn roots_label(roots: &[PathBuf]) -> String {
    roots
        .iter()
        .map(|r| output::escape(&r.display().to_string()))
        .collect::<Vec<_>>()
        .join(" ")
}

fn scan_roots(a: &ScanArgs, loaded: &freespace_core::config::Loaded) -> Vec<PathBuf> {
    if !a.roots.is_empty() {
        a.roots.clone()
    } else if !loaded.config.roots.is_empty() {
        loaded.config.roots.clone()
    } else {
        platform::default_roots(&platform::mountinfo())
    }
}

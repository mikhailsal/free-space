//! A full coarse scan: find hardlink owners, fold the tree, export, write, publish.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::time::Instant;

use super::group::walk_group;
use super::{ScanState, floor_of, precheck, record_failure, record_success, warnings, write_tmp};
use crate::coarse::export::{self, Built};
use crate::coarse::merge;
use crate::coarse::model::Model;
use crate::coarse::roots;
use crate::error::{Error, Result};
use crate::fault;
use crate::job::{Previous, ScanReport, expected_entries, has_required_schema, next_generation};
use crate::platform::{self, MountInfo};
use crate::snapshot::progress::{self, ProgressCallback, ProgressWriter};
use crate::snapshot::reader::Snapshot;
use crate::snapshot::writer::{Reserve, WriteMeta, WriteStats};
use crate::snapshot::{SnapshotDir, fsutil, lock, publish};

pub struct ScanRequest {
    pub roots: Vec<PathBuf>,
    /// Entries below this many parts per million of their filesystem's size are folded.
    pub ppm: u32,
    pub dir: SnapshotDir,
    pub keep_previous: bool,
    pub on_progress: Option<ProgressCallback>,
}

pub struct ScanOutcome {
    /// `entries` counts everything the walk saw; `snapshot_bytes` is what is left of it.
    pub report: ScanReport,
    /// Rows the snapshot keeps: retained entries and one row per non-empty bucket.
    pub rows: u64,
    /// Entries smaller than this many bytes are folded into `[small items]` rows.
    pub folded_below: u64,
}

pub fn run_scan(req: ScanRequest) -> Result<ScanOutcome> {
    if !(1..=1_000_000).contains(&req.ppm) {
        return Err(Error::usage(
            "the folding threshold must be between 1 and 1000000 parts per million",
        ));
    }
    let label = req
        .roots
        .iter()
        .map(|r| r.display().to_string())
        .collect::<Vec<_>>()
        .join(" ");
    fsutil::ensure_dir(req.dir.as_path())
        .map_err(|e| lock::open_error_with_path_arg("scan", None, req.dir.as_path(), e))?;
    let _lock = lock::acquire_with_path_arg(&req.dir, "scan", &label, None)?;
    let dir = req.dir.clone();
    let outcome = scan_locked(req, &label);
    if let Err(e) = &outcome {
        record_failure(&dir, "scan", e);
    }
    outcome
}

fn scan_locked(mut req: ScanRequest, label: &str) -> Result<ScanOutcome> {
    let t_all = Instant::now();
    publish::cleanup_tmp(&req.dir);
    let started_at = platform::now_secs();
    let mounts = platform::mountinfo();
    let roots = roots::prepare(&req.roots, req.ppm, &mounts)?;
    let paths: Vec<PathBuf> = roots.iter().map(|r| r.path.clone()).collect();
    let seen = Arc::new(AtomicU64::new(0));
    let pw = ProgressWriter::start(
        &req.dir,
        "scan",
        label,
        seen.clone(),
        expected_entries(&paths),
        req.on_progress.take(),
    );
    let mut model = Model {
        roots,
        nodes: Vec::new(),
        ppm: req.ppm,
    };
    let t = Instant::now();
    walk_roots(&mut model, &mounts, &seen, &pw)?;
    let walk_seconds = t.elapsed().as_secs_f64();
    fault::point(&req.dir, "walked");

    let t = Instant::now();
    merge::verify(&model)?;
    let built = export::build(&model, started_at)?;
    let finalize_seconds = t.elapsed().as_secs_f64();
    let rows = built.fin.order.len() as u64;
    let folded = floor_of(&model.roots);

    let published = publish_scan(&req, built, started_at, walk_seconds, pw)?;
    let total_seconds = t_all.elapsed().as_secs_f64();
    record_success(
        &req.dir,
        published.generation,
        warnings(&published.denied, folded),
        Some(ScanState {
            prev_generation: published.prev_generation,
            seconds: total_seconds,
        }),
    )?;
    Ok(ScanOutcome {
        report: ScanReport {
            generation: published.generation,
            prev_generation: published.prev_generation,
            entries: model.entries(),
            walk_seconds,
            finalize_seconds,
            write: published.write,
            publish_seconds: published.publish_seconds,
            total_seconds,
            snapshot_bytes: published.write.bytes,
            denied: published.denied.into(),
            errors: model.errors(),
        },
        rows,
        folded_below: folded,
    })
}

/// Both passes over every root. Hardlink owners are shared by all roots, so a file linked
/// from two roots is counted once, as a v1 scan counts it.
fn walk_roots(
    model: &mut Model,
    mounts: &[MountInfo],
    seen: &AtomicU64,
    pw: &ProgressWriter,
) -> Result<()> {
    for walked in walk_group(&model.roots, mounts, seen, Some(pw))? {
        model.nodes.extend(walked.nodes);
    }
    Ok(())
}

struct Published {
    generation: u64,
    prev_generation: Option<u64>,
    write: WriteStats,
    denied: Vec<String>,
    publish_seconds: f64,
}

fn publish_scan(
    req: &ScanRequest,
    built: Built,
    started_at: i64,
    walk_seconds: f64,
    pw: ProgressWriter,
) -> Result<Published> {
    pw.set_stage(progress::STAGE_WRITE);
    let rows = built.fin.order.len() as u64;
    precheck(&req.dir, rows, if req.keep_previous { 3 } else { 2 })?;
    fault::point(&req.dir, "prechecked");
    let (generation, current) = next_generation(&req.dir);
    let current = current.filter(|_| {
        Snapshot::open(&req.dir.current())
            .ok()
            .is_some_and(|snapshot| has_required_schema(&snapshot))
    });
    let previous = Previous::for_scan(&req.dir, req.keep_previous, current);
    let prev_generation = previous.generation();
    let generation = generation.max(prev_generation.map_or(1, |g| g + 1));
    let mut meta = WriteMeta::new(generation, prev_generation, started_at);
    meta.scan_seconds = Some(walk_seconds);
    let tmp = req.dir.tmp(generation);
    let (write, denied) = write_tmp(&tmp, built, &meta, &Reserve::new(req.dir.as_path()))?;
    fault::point(&req.dir, "written");

    pw.set_stage(progress::STAGE_PUBLISH);
    let t = Instant::now();
    if matches!(previous, Previous::Rotate(_)) {
        publish::rotate_prev(&req.dir)?;
    }
    publish::publish_current(&req.dir, &tmp)?;
    if matches!(previous, Previous::Drop) {
        publish::drop_prev(&req.dir);
    }
    Ok(Published {
        generation,
        prev_generation,
        write,
        denied,
        publish_seconds: t.elapsed().as_secs_f64(),
    })
}

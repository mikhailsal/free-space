//! A coarse refresh: rescan the smallest part of the snapshot that holds the path, merge
//! the result back and republish. Everything else is carried over from the current
//! snapshot, so a refresh costs what the rescanned part costs, not what the disk holds.

use std::ffi::OsStr;
use std::io::ErrorKind;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::time::Instant;

use super::{floor_of, precheck, record_failure, record_success, warnings, write_tmp};
use crate::coarse::model::{Model, invariant};
use crate::coarse::plan::{Plan, Slice, plan, scope};
use crate::coarse::walk::{self, Boundary, Walk};
use crate::coarse::{export, import, merge};
use crate::error::{Error, ErrorCode, Result};
use crate::fault;
use crate::job::next_generation;
use crate::platform::{self, MountInfo};
use crate::queries::common::{lossy, normalize, shell_path_word};
use crate::refresh::{RefreshReport, mounts, refresh_meta};
use crate::scan::RootInfo;
use crate::scan::walk::statvfs_safe;
use crate::snapshot::progress::{self, ProgressCallback, ProgressWriter};
use crate::snapshot::reader::Snapshot;
use crate::snapshot::writer::Reserve;
use crate::snapshot::{SnapshotDir, lock, publish};

pub struct RefreshRequest {
    pub path: Vec<u8>,
    pub dir: SnapshotDir,
    pub on_progress: Option<ProgressCallback>,
}

/// How much of the snapshot a refresh rescanned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    /// One bucket of small items of a retained folder.
    Bucket,
    /// A retained folder with everything below it.
    Folder,
    /// A whole scan root: asked for, or because the part to rescan holds hardlinks, whose
    /// sizes depend on folders outside it.
    Root,
}

#[derive(Clone, Debug)]
pub struct Rescanned {
    pub path: PathBuf,
    pub part: Part,
}

pub struct RefreshOutcome {
    /// `subtree_entries` counts what was rescanned, which can be more than the requested
    /// path: folded entries are rescanned together with the bucket that holds them.
    pub report: RefreshReport,
    pub rescanned: Rescanned,
}

pub fn run_refresh(req: RefreshRequest) -> Result<RefreshOutcome> {
    let path = normalize(&req.path)?;
    let label = lossy(&path);
    let path_arg = shell_path_word(&path);
    let _lock = lock::acquire_with_path_arg(&req.dir, "refresh", &label, path_arg.as_deref())?;
    let dir = req.dir.clone();
    let outcome = refresh_locked(req, &path, &label);
    if let Err(e) = &outcome {
        record_failure(&dir, "refresh", e);
    }
    outcome
}

fn refresh_locked(mut req: RefreshRequest, path: &[u8], label: &str) -> Result<RefreshOutcome> {
    let t_all = Instant::now();
    publish::cleanup_tmp(&req.dir);
    let snap = Snapshot::open_current(&req.dir)?;
    let t = Instant::now();
    let mut model = import::load(&snap)?;
    let load_seconds = t.elapsed().as_secs_f64();
    let target = PathBuf::from(OsStr::from_bytes(path));
    require_inside(&model, &target, label)?;
    let started = platform::now_secs();
    let live = inspect(&target, &model, snap.meta.one_file_system, label)?;
    let plan = locate(&mut model, &target, &live)?;
    let seen = Arc::new(AtomicU64::new(0));
    let pw = ProgressWriter::start(
        &req.dir,
        "refresh",
        label,
        seen.clone(),
        Some(plan.entries(&model.nodes)),
        req.on_progress.take(),
    );

    let t = Instant::now();
    let done = execute(&mut model, plan, &platform::mountinfo(), &seen)?;
    let walk_seconds = t.elapsed().as_secs_f64();
    let t = Instant::now();
    update_fs_stats(&mut model, &target);
    merge::verify(&model)?;
    let built = export::build(&model, started)?;
    let finalize_seconds = t.elapsed().as_secs_f64();

    pw.set_stage(progress::STAGE_WRITE);
    let entries = built.fin.order.len() as u64;
    precheck(&req.dir, entries, 2)?;
    fault::point(&req.dir, "prechecked");
    let (generation, _) = next_generation(&req.dir);
    let meta = refresh_meta(&snap, generation, path, started)?;
    drop(snap);
    let tmp = req.dir.tmp(generation);
    let (write, denied) = write_tmp(&tmp, built, &meta, &Reserve::new(req.dir.as_path()))?;
    fault::point(&req.dir, "written");

    pw.set_stage(progress::STAGE_PUBLISH);
    let t = Instant::now();
    publish::publish_current(&req.dir, &tmp)?;
    let publish_seconds = t.elapsed().as_secs_f64();
    drop(pw);

    record_success(
        &req.dir,
        generation,
        warnings(&denied, floor_of(&model.roots)),
        None,
    )?;
    Ok(RefreshOutcome {
        report: RefreshReport {
            generation,
            path: label.to_string(),
            entries,
            subtree_entries: done.visited,
            removed: matches!(live, Live::Gone { .. }),
            load_seconds,
            walk_seconds,
            finalize_seconds,
            write,
            publish_seconds,
            total_seconds: t_all.elapsed().as_secs_f64(),
            snapshot_bytes: write.bytes,
            denied,
        },
        rescanned: done.rescanned,
    })
}

fn require_inside(model: &Model, target: &Path, label: &str) -> Result<()> {
    if model.root_of(target).is_some() {
        return Ok(());
    }
    let names: Vec<String> = model
        .roots
        .iter()
        .map(|r| r.path.display().to_string())
        .collect();
    Err(Error::new(
        ErrorCode::PathNotFound,
        format!(
            "{label} is outside the scanned roots ({})",
            names.join(", ")
        ),
    )
    .with_fix(format!("use a path under {}", names.join(" or "))))
}

/// What is on disk at the requested path.
enum Live {
    Here,
    /// The path is gone; `missing` is the topmost part of it that does not exist, the
    /// first component below its deepest ancestor that still does.
    Gone {
        missing: PathBuf,
    },
}

fn inspect(target: &Path, model: &Model, one_file_system: bool, label: &str) -> Result<Live> {
    let roots: Vec<RootInfo> = model
        .roots
        .iter()
        .map(|r| RootInfo {
            path: r.path.clone(),
            dev: 0,
            fs_type: r.fs_type.clone(),
            stats: r.stats,
        })
        .collect();
    // Only a missing path can remove entries from the snapshot. Other stat failures (an
    // unsearchable ancestor, say) leave it intact.
    match target.symlink_metadata() {
        Ok(m) if !m.is_dir() => Err(not_a_folder(target, label)),
        Ok(m) => {
            mounts::check(target, Some(m.dev()), &roots, one_file_system, label)?;
            real_path(target, label)?;
            Ok(Live::Here)
        }
        Err(e) if e.kind() == ErrorKind::NotFound => {
            mounts::check(target, None, &roots, one_file_system, label)?;
            vanished(target, model, label)
        }
        Err(e) => Err(Error::io(format!("stat {label}"), e)),
    }
}

fn not_a_folder(target: &Path, label: &str) -> Error {
    let parent = target.parent().unwrap_or(Path::new("/"));
    let fix = match shell_path_word(parent.as_os_str().as_bytes()) {
        Some(parent) => format!("sudo freespace refresh {parent}"),
        None => "refresh the parent folder with sudo using its original path bytes".to_string(),
    };
    Error::usage(format!("{label} is not a folder on disk")).with_fix(fix)
}

fn real_path(target: &Path, label: &str) -> Result<()> {
    let real =
        std::fs::canonicalize(target).map_err(|e| Error::io(format!("resolve {label}"), e))?;
    if real != target {
        return Err(Error::usage(format!(
            "{label} resolves to {} on disk (symlink?); refresh the real path",
            real.display()
        )));
    }
    Ok(())
}

fn vanished(target: &Path, model: &Model, label: &str) -> Result<Live> {
    let root = model
        .root_of(target)
        .ok_or_else(|| invariant("path outside every scan root"))?;
    let mut missing = target;
    let live = loop {
        let parent = missing.parent().unwrap_or(Path::new("/"));
        if parent.symlink_metadata().is_ok_and(|m| m.is_dir()) || parent == missing {
            break parent;
        }
        missing = parent;
    };
    // a scan root that was inside the missing part is gone with it
    let gone = if live.starts_with(&root.path) {
        model.roots.iter().find(|r| r.path.starts_with(missing))
    } else {
        Some(root)
    };
    if let Some(gone) = gone {
        return Err(Error::usage(format!(
            "root {} no longer exists, so {label} cannot be refreshed; run a full scan",
            gone.path.display()
        )));
    }
    Ok(Live::Gone {
        missing: missing.to_path_buf(),
    })
}

/// The part of the snapshot to rescan. A path that is gone takes every retained entry
/// below it along, then its folder (or the bucket that held it) is rescanned.
fn locate(model: &mut Model, target: &Path, live: &Live) -> Result<Plan> {
    let Live::Gone { missing, .. } = live else {
        return plan(&model.nodes, target);
    };
    let dropped_links = model
        .nodes
        .iter()
        .find(|n| n.path == *missing)
        .map_or(0, |n| n.total.links);
    model.nodes.retain(|n| !n.path.starts_with(missing));
    if dropped_links > 0 {
        // their bytes may belong to a link elsewhere now: only a full pass can tell
        let root = model
            .root_of(missing)
            .ok_or_else(|| invariant("path outside every scan root"))?;
        return Ok(Plan::Retained(root.path.clone()));
    }
    plan(&model.nodes, missing)
}

struct Done {
    visited: u64,
    rescanned: Rescanned,
}

fn execute(model: &mut Model, plan: Plan, mounts: &[MountInfo], seen: &AtomicU64) -> Result<Done> {
    let root = model
        .root_of(plan.region())
        .map(|r| r.path.clone())
        .ok_or_else(|| invariant("region outside every scan root"))?;
    if plan.links(&model.nodes) > 0 {
        return rescan_root(model, &root, 0, mounts, seen);
    }
    match plan {
        Plan::Retained(path) if path == root => rescan_root(model, &root, 0, mounts, seen),
        Plan::Retained(path) => rescan_folder(model, &root, path, mounts, seen),
        Plan::Range(slice) => rescan_bucket(model, &root, &slice, mounts, seen),
    }
}

fn boundary<'a>(path: &Path, mounts: &'a [MountInfo]) -> Result<Boundary<'a>> {
    Boundary::of(path, mounts).map_err(|e| Error::io(format!("stat {}", path.display()), e))
}

fn rescan_root(
    model: &mut Model,
    root: &Path,
    spent: u64,
    mounts: &[MountInfo],
    seen: &AtomicU64,
) -> Result<Done> {
    let threshold = merge::threshold_for(&model.roots, root)?;
    let boundary = boundary(root, mounts)?;
    let w = Walk {
        boundary: &boundary,
        threshold,
        scope: None,
        seen,
    };
    let (walked, _) = walk::scan(root, true, &w)?;
    let visited = spent + walked.visited;
    merge::replace_root(model, root, walked.nodes)?;
    Ok(Done {
        visited,
        rescanned: Rescanned {
            path: root.to_path_buf(),
            part: Part::Root,
        },
    })
}

fn rescan_folder(
    model: &mut Model,
    root: &Path,
    path: PathBuf,
    mounts: &[MountInfo],
    seen: &AtomicU64,
) -> Result<Done> {
    let threshold = merge::threshold_for(&model.roots, &path)?;
    let boundary = boundary(&path, mounts)?;
    let w = Walk {
        boundary: &boundary,
        threshold,
        scope: None,
        seen,
    };
    let (walked, links) = walk::scan(&path, false, &w)?;
    if links {
        return rescan_root(model, root, walked.visited, mounts, seen);
    }
    let visited = walked.visited;
    merge::update(model, &path, walked.nodes)?;
    Ok(Done {
        visited,
        rescanned: Rescanned {
            path,
            part: Part::Folder,
        },
    })
}

fn rescan_bucket(
    model: &mut Model,
    root: &Path,
    slice: &Slice,
    mounts: &[MountInfo],
    seen: &AtomicU64,
) -> Result<Done> {
    let threshold = merge::threshold_for(&model.roots, &slice.parent)?;
    let boundary = boundary(&slice.parent, mounts)?;
    let range = scope(&model.nodes, &slice.parent, slice.lo, slice.hi);
    let w = Walk {
        boundary: &boundary,
        threshold,
        scope: Some(&range),
        seen,
    };
    let (walked, links) = walk::scan(&slice.parent, false, &w)?;
    if walked.unreadable {
        // the folder cannot be listed any more: it is kept as an unreadable folder
        return rescan_folder(model, root, slice.parent.clone(), mounts, seen);
    }
    if links {
        return rescan_root(model, root, walked.visited, mounts, seen);
    }
    let visited = walked.visited;
    merge::update_range(model, &slice.parent, &range, walked)?;
    Ok(Done {
        visited,
        rescanned: Rescanned {
            path: slice.parent.clone(),
            part: Part::Bucket,
        },
    })
}

/// A refresh usually follows a cleanup: the free space shown "at scan" should move with it.
fn update_fs_stats(model: &mut Model, target: &Path) {
    let Some(root) = model
        .roots
        .iter_mut()
        .filter(|r| target.starts_with(&r.path))
        .max_by_key(|r| r.path.as_os_str().len())
    else {
        return;
    };
    if statvfs_safe(&root.fs_type)
        && let Ok(stats) = platform::statvfs(&root.path)
    {
        root.stats = stats;
    }
}

#[cfg(test)]
mod tests;

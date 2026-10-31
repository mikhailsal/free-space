//! Experimental disk-backed implementation of the existing refresh command.
use super::*;
use crate::scan::staged::StagedTree;

pub(super) fn run(req: RefreshRequest, path: Vec<u8>, label: &str) -> Result<RefreshReport> {
    let t_all = Instant::now();
    publish::cleanup_tmp(&req.dir);
    let snap = Snapshot::open_current(&req.dir)?;
    let (x, xpath) = resolve(&snap, &path)?;
    check_target(&x, label)?;
    let started = platform::now_secs();
    let seen = Arc::new(AtomicU64::new(0));
    let pw = ProgressWriter::start(
        &req.dir,
        "refresh",
        label,
        seen.clone(),
        Some(x.items as u64),
        req.on_progress,
    );
    let t = Instant::now();
    let roots = load::roots(&snap)?;
    let roots_seconds = t.elapsed().as_secs_f64();
    let t = Instant::now();
    let sub = rescan_subtree(&snap, &roots, &x, &xpath, req.workers, seen, label)?;
    let walk_seconds = t.elapsed().as_secs_f64();
    let removed = sub.is_none();
    let subtree_entries = sub.as_ref().map_or(0, |tree| tree.nodes.len() as u64);
    stage_precheck(&req.dir, snap.meta.entries, subtree_entries)?;
    let mut root_tree = Tree {
        roots,
        ..Tree::default()
    };
    update_fs_stats(&mut root_tree, &xpath);
    let t = Instant::now();
    let stage = StagedTree::refresh(
        &snap,
        root_tree.roots,
        x.id,
        x.subtree_end,
        sub,
        &req.dir.as_path().join("snapshot.stage.tmp"),
        &Reserve::new(req.dir.as_path()),
    )?;
    let load_seconds = roots_seconds + t.elapsed().as_secs_f64();
    pw.set_stage(progress::STAGE_WRITE);
    precheck::run(&req.dir, stage.len(), 2)?;
    crate::fault::point(&req.dir, "prechecked");
    let (generation, _) = crate::job::next_generation(&req.dir);
    let meta = refresh_meta(&snap, generation, &xpath, started)?;
    let tmp = req.dir.tmp(generation);
    let entries = stage.len();
    drop(snap);
    let (ws, denied, _errors) = stage.write(&tmp, &meta)?;
    let denied = collect_denied(denied, &tmp)?;
    crate::fault::point(&req.dir, "written");
    pw.set_stage(progress::STAGE_PUBLISH);
    let t = Instant::now();
    publish::publish_current(&req.dir, &tmp)?;
    let publish_seconds = t.elapsed().as_secs_f64();
    drop(pw);
    record_success(&req.dir, generation, &denied)?;
    Ok(RefreshReport {
        generation,
        path: label.to_string(),
        entries,
        subtree_entries,
        removed,
        load_seconds,
        walk_seconds,
        finalize_seconds: 0.0,
        write: ws,
        publish_seconds,
        total_seconds: t_all.elapsed().as_secs_f64(),
        snapshot_bytes: ws.bytes,
        denied,
    })
}

// Six estimates are an allowance, not an upper bound; live guards still apply.
fn stage_precheck(dir: &SnapshotDir, old_entries: u64, subtree_entries: u64) -> Result<()> {
    let estimated = old_entries
        .checked_add(subtree_entries.saturating_sub(1))
        .ok_or_else(|| {
            let mut error = Error::new(ErrorCode::Space, "refresh entry estimate overflow");
            error.last_error = Some(state::LastError {
                ts: platform::now_secs(),
                op: "precheck".into(),
                message: error.message.clone(),
            });
            error
        })?;
    precheck::run(dir, estimated, 6)?;
    Ok(())
}

fn collect_denied(denied: crate::scan::denied::DeniedPaths, tmp: &Path) -> Result<Vec<String>> {
    match denied.into_paths() {
        Ok(paths) => Ok(paths),
        Err(error) => {
            let _ = std::fs::remove_file(tmp);
            Err(error)
        }
    }
}

#[cfg(test)]
#[path = "staged/tests.rs"]
mod tests;

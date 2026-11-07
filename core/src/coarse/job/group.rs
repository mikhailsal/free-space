//! Both walk passes over a set of roots at once. The roots share one hardlink owner table,
//! so a file linked from two of them is counted once, as a v1 scan counts it.

use std::sync::atomic::{AtomicU64, Ordering};

use crate::coarse::model::RootMeta;
use crate::coarse::walk::{Boundary, Owners, Walk, Walked, fold, link_owners};
use crate::error::{Error, Result};
use crate::platform::MountInfo;
use crate::snapshot::progress::{self, ProgressWriter};

pub(super) fn walk_of<'a>(
    boundary: &'a Boundary<'a>,
    threshold: u64,
    seen: &'a AtomicU64,
) -> Walk<'a> {
    Walk {
        boundary,
        threshold,
        scope: None,
        seen,
    }
}

fn stage(pw: Option<&ProgressWriter>, stage: u8) {
    if let Some(pw) = pw {
        pw.set_stage(stage);
    }
}

/// What the walk of each of `roots` found, in the order of `roots`. Hardlinks cannot cross
/// a filesystem, so what must share its owners is the roots of one filesystem; a scan
/// passes all of its roots because it has no way to tell them apart cheaply.
pub(super) fn walk_group(
    roots: &[RootMeta],
    mounts: &[MountInfo],
    seen: &AtomicU64,
    pw: Option<&ProgressWriter>,
) -> Result<Vec<Walked>> {
    let boundaries = roots
        .iter()
        .map(|r| {
            Boundary::of(&r.path, mounts)
                .map_err(|e| Error::io(format!("stat {}", r.path.display()), e))
        })
        .collect::<Result<Vec<_>>>()?;
    let mut owners = Owners::new();
    stage(pw, progress::STAGE_LINKS);
    for (root, boundary) in roots.iter().zip(&boundaries) {
        link_owners(
            &root.path,
            &walk_of(boundary, root.threshold, seen),
            &mut owners,
        );
    }
    // progress counts the second pass alone, which is the one that ends in a result
    seen.store(0, Ordering::Relaxed);
    stage(pw, progress::STAGE_WALK);
    roots
        .iter()
        .zip(&boundaries)
        .map(|(root, boundary)| {
            let w = walk_of(boundary, root.threshold, seen);
            fold(&root.path, true, &w, &mut owners)
        })
        .collect()
}

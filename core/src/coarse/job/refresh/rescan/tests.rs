use std::fs;

use super::*;
use crate::coarse::testing::{KIB, model_of, nested_model, put};
use crate::error::ErrorCode;

const THRESHOLD: u64 = 100 * KIB;

/// `top` with a folder `a/m` that is also a root of its own. Without privileges both lie
/// on one filesystem, which is what a nested root whose filesystem was unmounted looks like.
fn unmounted(top: &Path) -> (Model, PathBuf) {
    put(top, "a/big.bin", 300 * 1024);
    put(top, "a/m/inner.bin", 200 * 1024);
    put(top, "z/other.bin", 150 * 1024);
    let inner = top.join("a/m");
    (nested_model(top, &inner, THRESHOLD), inner)
}

#[test]
fn a_nested_root_that_is_no_longer_a_mount_point_stops_every_rescan_that_reaches_it() {
    let tmp = tempfile::tempdir().unwrap();
    let top = tmp.path();
    let (model, inner) = unmounted(top);
    let named = format!("{} is no longer a mount point", inner.display());
    // the outer root, the folder that holds the mount point and the nested root itself
    for within in [top.to_path_buf(), top.join("a"), inner.clone()] {
        let refused = check_nested(&model, &within).unwrap_err();
        assert_eq!(refused.code, ErrorCode::Usage, "{}", within.display());
        assert!(refused.message.contains(&named), "{refused:?}");
        assert!(refused.message.contains("run a full scan"), "{refused:?}");
    }
}

#[test]
fn a_rescan_that_does_not_reach_a_nested_root_is_not_held_up_by_it() {
    let tmp = tempfile::tempdir().unwrap();
    let top = tmp.path();
    let (model, inner) = unmounted(top);
    check_nested(&model, &top.join("z")).unwrap();
    check_nested(&model, &inner.join("below")).unwrap();
    // a snapshot with one root has nothing nested
    check_nested(&model_of(top, THRESHOLD), top).unwrap();
}

#[test]
fn a_nested_root_that_is_gone_from_disk_is_left_to_the_walk() {
    let tmp = tempfile::tempdir().unwrap();
    let top = tmp.path();
    let (model, inner) = unmounted(top);
    fs::remove_dir_all(&inner).unwrap();
    check_nested(&model, top).unwrap();
}

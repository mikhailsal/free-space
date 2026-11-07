use std::fs;

use super::*;
use crate::coarse::model::RootMeta;
use crate::platform::FsStats;

fn model_with(roots: &[&Path]) -> Model {
    Model {
        roots: roots
            .iter()
            .map(|path| RootMeta {
                path: path.to_path_buf(),
                fs_type: String::new(),
                stats: FsStats::default(),
                threshold: 1,
            })
            .collect(),
        nodes: Vec::new(),
        ppm: 1000,
    }
}

#[test]
fn a_path_that_is_gone_names_the_topmost_part_that_is_missing() {
    let tmp = tempfile::tempdir().unwrap();
    let top = tmp.path().join("top");
    fs::create_dir_all(top.join("a")).unwrap();
    let model = model_with(&[&top]);
    match vanished(&top.join("a/b/c"), &model, "label").unwrap() {
        Live::Gone { missing } => assert_eq!(missing, top.join("a/b")),
        Live::Here => panic!("the path is gone"),
    }
}

#[test]
fn a_scan_root_inside_the_missing_part_is_gone_with_it() {
    let tmp = tempfile::tempdir().unwrap();
    let top = tmp.path().join("top");
    fs::create_dir_all(&top).unwrap();
    // `top/gone/inner` was a root of its own, below a folder that no longer exists
    let inner = top.join("gone/inner");
    let model = model_with(&[&top, &inner]);
    let Err(error) = vanished(&top.join("gone/other"), &model, "the label") else {
        panic!("a root went with the folder");
    };
    assert_eq!(error.code, ErrorCode::Usage);
    let says = format!("root {} no longer exists", inner.display());
    assert!(error.message.contains(&says), "{}", error.message);
    assert!(
        error.message.contains("run a full scan"),
        "{}",
        error.message
    );
}

#[test]
fn the_root_of_the_path_being_gone_is_refused_as_before() {
    let tmp = tempfile::tempdir().unwrap();
    let top = tmp.path().join("top");
    let model = model_with(&[&top]);
    let Err(error) = vanished(&top.join("a"), &model, "the label") else {
        panic!("the root itself is missing");
    };
    let says = format!("root {} no longer exists", top.display());
    assert!(error.message.contains(&says), "{}", error.message);
}

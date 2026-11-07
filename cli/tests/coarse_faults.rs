//! The binary dies at each publish step of a coarse scan or refresh (kill -9, as in
//! `faults.rs`). Debug builds only: the steps are `freespace_core::fault` points driven by
//! FREESPACE_FAULT.

#![cfg(debug_assertions)]

mod common;

use std::fs;
use std::os::unix::process::ExitStatusExt;
use std::path::Path;
use std::process::Output;

use common::Env;
use serde_json::Value;

const STEPS: [&str; 6] = [
    "walked",
    "prechecked",
    "written",
    "linked",
    "rotated",
    "published",
];

fn s(p: &Path) -> &str {
    p.to_str().unwrap()
}

fn ok(e: &Env, args: &[&str]) -> Output {
    let o = e.run(args);
    assert!(
        o.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    o
}

/// Two published generations of a small tree, then a change for the third.
fn with_two_generations() -> Env {
    let e = Env::new();
    let r = e.root();
    fs::create_dir_all(r.join("a/b")).unwrap();
    fs::write(r.join("a/b/one"), vec![1u8; 64 << 10]).unwrap();
    fs::write(r.join("top"), vec![1u8; 8 << 10]).unwrap();
    ok(&e, &["--backend", "coarse", "scan", s(&r)]);
    fs::write(r.join("a/two"), vec![1u8; 128 << 10]).unwrap();
    ok(&e, &["--backend", "coarse", "scan", s(&r)]);
    fs::write(r.join("a/b/three"), vec![1u8; 256 << 10]).unwrap();
    e
}

fn json(e: &Env, args: &[&str]) -> (i32, Value) {
    let o = e.run(args);
    let v = serde_json::from_slice(&o.stdout)
        .unwrap_or_else(|x| panic!("{args:?}: {x}: {}", String::from_utf8_lossy(&o.stdout)));
    (o.status.code().unwrap(), v)
}

fn generation(e: &Env) -> u64 {
    let (code, v) = json(e, &["--backend", "coarse", "status", "--json"]);
    assert_eq!(code, 0, "{v}");
    v["generation"].as_u64().unwrap()
}

fn leftovers(e: &Env) -> Vec<String> {
    fs::read_dir(e.snap().join("coarse"))
        .unwrap()
        .flatten()
        .map(|x| x.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".tmp") || n.starts_with(".pause-"))
        .collect()
}

#[test]
fn a_kill_at_any_publish_step_leaves_a_readable_coarse_snapshot_and_a_clean_retry() {
    for step in STEPS {
        let e = with_two_generations();
        let o = e
            .cmd(&["--backend", "coarse", "scan", s(&e.root())])
            .env("FREESPACE_FAULT", step)
            .output()
            .unwrap();
        assert_eq!(o.status.signal(), Some(9), "{step}: killed at the step");

        // the published snapshot is whole: old before the rename, new after
        let want = if step == "published" { 3 } else { 2 };
        assert_eq!(generation(&e), want, "{step}");
        let (code, ls) = json(&e, &["--backend", "coarse", "ls", s(&e.root()), "--json"]);
        assert_eq!(code, 0, "{step}: {ls}");
        assert_eq!(ls["generation"], want, "{step}");

        // the lock died with the process; the next scan cleans up and publishes
        ok(&e, &["--backend", "coarse", "scan", s(&e.root())]);
        assert_eq!(generation(&e), want + 1, "{step}");
        assert!(leftovers(&e).is_empty(), "{step}: {:?}", leftovers(&e));
        // nothing of it leaked into the directory of the default backend
        assert!(!e.snap().join("snapshot.db").exists(), "{step}");
    }
}

#[test]
fn a_kill_during_a_coarse_refresh_keeps_current_and_prev() {
    for step in ["prechecked", "written", "published"] {
        let e = with_two_generations();
        let target = e.root().join("a");
        let o = e
            .cmd(&["--backend", "coarse", "refresh", s(&target)])
            .env("FREESPACE_FAULT", step)
            .output()
            .unwrap();
        assert_eq!(o.status.signal(), Some(9), "{step}");
        let want = if step == "published" { 3 } else { 2 };
        assert_eq!(generation(&e), want, "{step}");
        // refresh never rotates: prev is still generation 1
        let (code, v) = json(&e, &["--backend", "coarse", "diff", s(&e.root()), "--json"]);
        assert_eq!(code, 0, "{step}: {v}");
        assert_eq!(v["prev_generation"], 1, "{step}");
        assert_eq!(v["partial"], step == "published", "{step}");
        ok(&e, &["--backend", "coarse", "refresh", s(&target)]);
        assert!(leftovers(&e).is_empty(), "{step}: {:?}", leftovers(&e));
    }
}

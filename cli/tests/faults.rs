//! The binary dies or pauses at each publish step (T10, Section 6 #1/#3,
//! "kill -9 at every publish step"). Debug builds only: the steps are
//! `freespace_core::fault` points driven by FREESPACE_FAULT / FREESPACE_PAUSE.

#![cfg(debug_assertions)]

use std::fs;
use std::os::unix::process::ExitStatusExt;
use std::path::Path;
use std::process::{Child, Output, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;

const STEPS: [&str; 6] = [
    "walked",
    "prechecked",
    "written",
    "linked",
    "rotated",
    "published",
];

mod common;

use common::Env;

impl Env {
    /// Two published generations of a small tree, then a change for the third.
    fn with_two_generations() -> Env {
        let e = Env::new();
        let r = e.root();
        fs::create_dir_all(r.join("a/b")).unwrap();
        fs::write(r.join("a/b/one"), vec![1u8; 64 << 10]).unwrap();
        fs::write(r.join("top"), vec![1u8; 8 << 10]).unwrap();
        e.ok(&["scan", s(&r)]);
        fs::write(r.join("a/two"), vec![1u8; 128 << 10]).unwrap();
        e.ok(&["scan", s(&r)]);
        fs::write(r.join("a/b/three"), vec![1u8; 256 << 10]).unwrap();
        e
    }

    fn ok(&self, args: &[&str]) -> Output {
        let o = self.run(args);
        assert!(
            o.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&o.stderr)
        );
        o
    }

    fn json(&self, args: &[&str]) -> (i32, Value) {
        let o = self.run(args);
        let v = serde_json::from_slice(&o.stdout)
            .unwrap_or_else(|e| panic!("{args:?}: {e}: {}", String::from_utf8_lossy(&o.stdout)));
        (o.status.code().unwrap(), v)
    }

    fn generation(&self) -> u64 {
        let (code, v) = self.json(&["status", "--json"]);
        assert_eq!(code, 0, "{v}");
        v["generation"].as_u64().unwrap()
    }

    /// `diff` pairs neighbours or refuses: never deltas across a skipped
    /// generation (Section 6 #3).
    fn diff_is_honest(&self) -> Option<(u64, u64)> {
        let (code, v) = self.json(&["diff", s(&self.root()), "--json"]);
        match code {
            0 => {
                let (g, p) = (
                    v["generation"].as_u64().unwrap(),
                    v["prev_generation"].as_u64().unwrap(),
                );
                assert_eq!(p + 1, g, "diff paired {p} with {g}: {v}");
                Some((p, g))
            }
            3 | 4 => {
                let c = v["error"]["code"].as_str().unwrap();
                assert!(c == "NO_PREV" || c == "ROTATED", "{v}");
                None
            }
            _ => panic!("diff exit {code}: {v}"),
        }
    }

    fn leftovers(&self) -> Vec<String> {
        fs::read_dir(self.snap())
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".tmp") || n.starts_with(".pause-"))
            .collect()
    }
}

fn s(p: &Path) -> &str {
    p.to_str().unwrap()
}

#[test]
fn a_kill_at_any_publish_step_leaves_a_readable_snapshot_and_a_clean_retry() {
    for low_memory in [false, true] {
        for step in STEPS {
            let e = Env::with_two_generations();
            if low_memory {
                fs::write(e.config(), "low_memory = true\n").unwrap();
            }
            let o = e
                .cmd(&["scan", s(&e.root())])
                .env("FREESPACE_FAULT", step)
                .output()
                .unwrap();
            assert_eq!(o.status.signal(), Some(9), "{step}: killed at the step");

            // the published snapshot is whole: old before the rename, new after
            let want = if step == "published" { 3 } else { 2 };
            assert_eq!(e.generation(), want, "{step}");
            let (code, ls) = e.json(&["ls", s(&e.root()), "--json"]);
            assert_eq!(code, 0, "{step}: {ls}");
            let paired = e.diff_is_honest();
            if step == "rotated" {
                // prev already holds what is still current: honest NO_PREV
                assert_eq!(paired, None, "{step}");
            } else {
                assert_eq!(paired, Some((want - 1, want)), "{step}");
            }

            // the lock died with the process; the next scan cleans up and publishes
            e.ok(&["scan", s(&e.root())]);
            assert_eq!(e.generation(), want + 1, "{step}");
            assert_eq!(e.diff_is_honest(), Some((want, want + 1)), "{step}");
            assert!(e.leftovers().is_empty(), "{step}: {:?}", e.leftovers());
        }
    }
}

fn wait_for_file(p: &Path, child: &mut Child) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !p.exists() {
        if let Some(st) = child.try_wait().unwrap() {
            panic!("scan exited ({st}) before {}", p.display());
        }
        assert!(Instant::now() < deadline, "no {}", p.display());
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn diff_during_each_publish_step_pairs_neighbours_or_refuses() {
    for step in STEPS {
        let e = Env::with_two_generations();
        let mut child = e
            .cmd(&["scan", s(&e.root())])
            .env("FREESPACE_PAUSE", step)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let pause = e.snap().join(format!(".pause-{step}"));
        wait_for_file(&pause, &mut child);

        let mid = e.diff_is_honest();
        let (code, st) = e.json(&["status", "--json"]);
        assert_eq!(code, 0);
        assert_eq!(st["scanning"]["op"], "scan", "{step}: {st}");
        // a second scan waits its turn instead of racing the publish
        let (code, busy) = e.json(&["scan", s(&e.root()), "--json"]);
        assert_eq!(code, 6, "{step}: {busy}");
        assert_eq!(busy["error"]["code"], "LOCK");

        fs::remove_file(&pause).unwrap();
        assert!(child.wait().unwrap().success(), "{step}");
        match step {
            "rotated" => assert_eq!(mid, None),
            "published" => assert_eq!(mid, Some((2, 3))),
            _ => assert_eq!(mid, Some((1, 2)), "{step}"),
        }
        assert_eq!(e.diff_is_honest(), Some((2, 3)), "{step}");
        assert!(e.leftovers().is_empty(), "{step}: {:?}", e.leftovers());
    }
}

#[test]
fn a_kill_during_refresh_keeps_current_and_prev() {
    for step in ["prechecked", "written", "published"] {
        let e = Env::with_two_generations();
        let o = e
            .cmd(&["refresh", s(&e.root().join("a"))])
            .env("FREESPACE_FAULT", step)
            .output()
            .unwrap();
        assert_eq!(o.status.signal(), Some(9), "{step}");
        let want = if step == "published" { 3 } else { 2 };
        assert_eq!(e.generation(), want, "{step}");
        // refresh never rotates: prev is still generation 1
        let (code, v) = e.json(&["diff", s(&e.root()), "--json"]);
        assert_eq!(code, 0, "{step}: {v}");
        assert_eq!(v["prev_generation"], 1, "{step}");
        assert_eq!(v["partial"], step == "published", "{step}");
        e.ok(&["refresh", s(&e.root().join("a"))]);
        assert!(e.leftovers().is_empty(), "{step}: {:?}", e.leftovers());
    }
}

//! The coarse backend through the real binary: its own snapshot directory, the opt-in by
//! flag or config, the answers that say what they leave out, and the refusals.

mod common;

use std::path::Path;

use common::Env;
use serde_json::Value;

const KIB: u64 = 1024;

fn answer(e: &Env, args: &[&str]) -> Value {
    let output = e.run(args);
    assert!(
        output.status.success(),
        "{args:?}: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

/// The error a failing command prints under `--json`.
fn refusal(e: &Env, args: &[&str]) -> Value {
    let output = e.run(args);
    assert!(!output.status.success(), "{args:?} should fail");
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    value["error"].clone()
}

/// `args` with the coarse backend selected.
fn selected<'a>(args: &[&'a str]) -> Vec<&'a str> {
    let mut all = vec!["--backend", "coarse"];
    all.extend_from_slice(args);
    all
}

/// A coarse command, answered as JSON.
fn coarse(e: &Env, args: &[&str]) -> Value {
    let mut all = selected(args);
    all.push("--json");
    answer(e, &all)
}

/// A coarse command that fails, and the error it prints.
fn coarse_refusal(e: &Env, args: &[&str]) -> Value {
    let mut all = selected(args);
    all.push("--json");
    refusal(e, &all)
}

/// What a coarse command prints on stderr.
fn coarse_notes(e: &Env, args: &[&str]) -> String {
    String::from_utf8_lossy(&e.run(&selected(args)).stderr).into_owned()
}

/// A coarse scan of `path` with the folding threshold `ppm`.
fn scan(e: &Env, path: &str, ppm: &str) -> Value {
    coarse(e, &["scan", path, "--coarse-ppm", ppm])
}

/// The `--coarse-ppm` that gives a threshold of about 192 KiB on the filesystem of `dir`,
/// and the threshold it gives.
fn sizing(dir: &Path) -> (u32, u64) {
    let total = u128::from(freespace_core::platform::statvfs(dir).unwrap().total);
    let ppm = (u128::from(192 * KIB) * 1_000_000)
        .div_ceil(total)
        .clamp(1, 1_000_000) as u32;
    let threshold = (total * u128::from(ppm)).div_ceil(1_000_000).max(1) as u64;
    (ppm, threshold)
}

fn write(path: &Path, len: u64) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut x = 0x9e37_79b9_7f4a_7c15u64;
    let bytes: Vec<u8> = (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 24) as u8
        })
        .collect();
    std::fs::write(path, bytes).unwrap();
}

/// A big file, a folder of small notes and a threshold to match. `None` where the
/// filesystem is so large that the fixture would be absurd.
fn fixture(e: &Env) -> Option<(u32, u64)> {
    std::fs::create_dir_all(e.root()).unwrap();
    let (ppm, threshold) = sizing(&e.root());
    if threshold > 64 * 1024 * KIB {
        return None;
    }
    write(&e.root().join("big/huge.bin"), 2 * threshold);
    for i in 0..10 {
        write(&e.root().join(format!("docs/n{i}.txt")), KIB);
    }
    Some((ppm, threshold))
}

fn text(path: &Path) -> String {
    path.to_str().unwrap().to_owned()
}

/// The first scan of the fixture: what it saw and where it put the snapshot.
fn assert_first_scan(e: &Env, first: &Value) {
    assert_eq!(first["generation"], 1);
    // the root, big, huge.bin, docs and its ten notes
    assert_eq!(first["entries"], 14);
    assert!(e.snap().join("coarse/snapshot.db").exists());
    assert!(!e.snap().join("snapshot.db").exists());
    let v1 = e.run(&["status", "--json"]);
    assert!(!v1.status.success(), "v1 has no snapshot");
}

#[test]
fn a_scan_folds_the_small_entries_and_the_answers_say_so() {
    let e = Env::new();
    let Some((ppm, threshold)) = fixture(&e) else {
        return;
    };
    let path = text(&e.root());
    assert_first_scan(&e, &scan(&e, &path, &ppm.to_string()));

    let ls = coarse(&e, &["ls", &path]);
    assert_eq!(ls["folded_below"], threshold);
    let entries = ls["entries"].as_array().unwrap();
    let folded: Vec<&Value> = entries.iter().filter(|x| x["folded"] == true).collect();
    assert_eq!(folded.len(), 1, "{ls}");
    assert_eq!(folded[0]["items"], 11);
    assert!(entries.iter().any(|x| x["name"] == "big"));
    let status = coarse(&e, &["status"]);
    assert_eq!(status["generation"], 1);
    let warnings = status["warnings"].to_string();
    assert!(warnings.contains("coarse snapshot"), "{warnings}");
}

#[test]
fn a_second_scan_and_a_refresh_publish_generations_that_every_query_reads() {
    let e = Env::new();
    let Some((ppm, threshold)) = fixture(&e) else {
        return;
    };
    let path = text(&e.root());
    let ppm = ppm.to_string();
    scan(&e, &path, &ppm);
    write(&e.root().join("more/huge2.bin"), 2 * threshold);
    let second = scan(&e, &path, &ppm);
    assert_eq!(second["generation"], 2);
    assert_eq!(second["prev_generation"], 1);
    let diff = coarse(&e, &["diff", &path]);
    let entries = diff["entries"].as_array().unwrap();
    let more = entries.iter().find(|x| x["name"] == "more").unwrap();
    assert_eq!(more["status"], "new", "{diff}");

    let more_dir = text(&e.root().join("more"));
    assert_eq!(coarse(&e, &["refresh", &more_dir])["generation"], 3);
    let status = coarse(&e, &["status"]);
    assert_eq!(status["prev_generation"], 1);
    assert_eq!(status["refreshed"].as_array().unwrap().len(), 1, "{status}");
    for command in ["ls", "top", "find", "diff"] {
        let value = coarse(&e, &[command, &path]);
        assert_eq!(value["generation"], 3, "{command}: {value}");
        assert_eq!(value["folded_below"], threshold, "{command}: {value}");
    }
}

#[test]
fn text_output_and_notes_say_what_is_folded() {
    let e = Env::new();
    let Some((ppm, _)) = fixture(&e) else { return };
    let path = text(&e.root());
    let scan = coarse_notes(&e, &["scan", &path, "--coarse-ppm", &ppm.to_string()]);
    assert!(scan.contains("generation 1 published"), "{scan}");
    assert!(scan.contains("coarse snapshot of"), "{scan}");
    assert!(scan.contains("folded into [small items] rows"), "{scan}");

    let ls = e.run(&selected(&["ls", &path]));
    let out = String::from_utf8_lossy(&ls.stdout);
    let err = String::from_utf8_lossy(&ls.stderr);
    assert!(out.contains("[small items"), "{out}");
    assert!(err.contains("folded into [small items] rows"), "{err}");

    let docs = text(&e.root().join("docs"));
    let note = coarse_notes(&e, &["refresh", &docs]);
    assert!(note.contains("small items"), "{note}");
    assert!(note.contains("that bucket was rescanned"), "{note}");

    let gone = text(&e.root().join("docs/n3.txt"));
    let missing = coarse_refusal(&e, &["ls", &gone]);
    assert_eq!(missing["code"], "PATH_NOT_FOUND");
    let message = missing["message"].as_str().unwrap();
    assert!(message.contains("[small items]"), "{missing}");
}

#[test]
fn config_opts_in_and_the_flag_overrides_it() {
    let e = Env::new();
    let Some((ppm, threshold)) = fixture(&e) else {
        return;
    };
    let path = text(&e.root());
    let config = format!("backend = \"coarse\"\ncoarse_ppm = {ppm}\n");
    std::fs::write(e.config(), config).unwrap();
    answer(&e, &["scan", &path, "--json"]);
    assert!(e.snap().join("coarse/snapshot.db").exists());
    assert!(!e.snap().join("snapshot.db").exists());
    assert_eq!(
        answer(&e, &["ls", &path, "--json"])["folded_below"],
        threshold
    );

    let double = (ppm * 2).to_string();
    answer(&e, &["scan", &path, "--coarse-ppm", &double, "--json"]);
    let wider = answer(&e, &["ls", &path, "--json"])["folded_below"]
        .as_u64()
        .unwrap();
    assert!(wider.abs_diff(2 * threshold) <= 1, "{wider} vs {threshold}");

    // another backend on the same snapshot directory is a different snapshot
    let v1 = answer(&e, &["--backend", "v1", "scan", &path, "--json"]);
    assert_eq!(v1["generation"], 1);
    let listed = answer(&e, &["--backend", "v1", "ls", &path, "--json"]);
    assert!(listed["folded_below"].is_null());
    assert_eq!(answer(&e, &["status", "--json"])["generation"], 2);
}

#[test]
fn flags_of_another_backend_are_mistakes_not_no_ops() {
    let e = Env::new();
    std::fs::create_dir_all(e.root()).unwrap();
    let path = text(&e.root());
    let cases: [&[&str]; 5] = [
        &["--backend", "v1", "scan", &path, "--coarse-ppm", "5"],
        &["--backend", "v2", "scan", &path, "--coarse-ppm", "5"],
        &["--backend", "coarse", "scan", &path, "--cross-mounts"],
        &["--backend", "coarse", "scan", &path, "--coarse-ppm", "0"],
        &[
            "--backend",
            "coarse",
            "scan",
            &path,
            "--coarse-ppm",
            "1000001",
        ],
    ];
    for args in cases {
        let mut all = args.to_vec();
        all.push("--json");
        let error = refusal(&e, &all);
        assert_eq!(error["code"], "USAGE", "{args:?}: {error}");
    }
    assert!(!e.snap().join("coarse/snapshot.db").exists());
}

#[test]
fn refresh_refuses_without_a_snapshot_and_outside_the_roots() {
    let e = Env::new();
    let Some((ppm, _)) = fixture(&e) else { return };
    let path = text(&e.root());
    let error = coarse_refusal(&e, &["refresh", &path]);
    assert_eq!(error["code"], "NO_SNAPSHOT", "{error}");

    scan(&e, &path, &ppm.to_string());
    let elsewhere = text(e.tmp.path());
    let error = coarse_refusal(&e, &["refresh", &elsewhere]);
    assert_eq!(error["code"], "PATH_NOT_FOUND", "{error}");
    let file = text(&e.root().join("docs/n1.txt"));
    let error = coarse_refusal(&e, &["refresh", &file]);
    assert_eq!(error["code"], "USAGE", "{error}");
    assert_eq!(coarse(&e, &["status"])["generation"], 1);
}

#[test]
fn installing_needs_root_before_anything_in_the_coarse_directory_is_touched() {
    if unsafe { libc::geteuid() } == 0 {
        return;
    }
    let e = Env::new();
    std::fs::create_dir_all(e.snap().join("coarse")).unwrap();
    let data = e.snap().join("coarse/snapshot.db");
    std::fs::write(&data, b"preserve coarse data").unwrap();
    for config in ["", "backend = \"coarse\"\n"] {
        std::fs::write(e.config(), config).unwrap();
        for uninstall in [false, true] {
            let mut args = vec!["install-timer"];
            if uninstall {
                args.push("--uninstall");
            }
            let error = coarse_refusal(&e, &args);
            assert_eq!(error["code"], "NOT_ROOT", "{error}");
            assert_eq!(std::fs::read_to_string(e.config()).unwrap(), config);
            assert_eq!(std::fs::read(&data).unwrap(), b"preserve coarse data");
        }
    }
}

mod common;

use common::Env;
use serde_json::Value;
use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf};

struct DeniedDirectories(Vec<PathBuf>);
impl Drop for DeniedDirectories {
    fn drop(&mut self) {
        for path in &self.0 {
            let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o700));
        }
    }
}

fn answer(env: &Env, args: &[&str]) -> Value {
    let result = env.run(args);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    serde_json::from_slice(&result.stdout).unwrap()
}

fn v2_stored_bytes(env: &Env) -> u64 {
    ["generations.db", "generations.db-wal"]
        .into_iter()
        .filter_map(|name| fs::metadata(env.snap().join("v2").join(name)).ok())
        .map(|metadata| metadata.len())
        .sum()
}

#[test]
fn write_reports_preserve_all_denied_paths_beyond_status_limit() {
    if freespace_core::snapshot::fsutil::is_root() {
        return; // Root can read mode-000 directories; ordinary-user CI exercises this fixture.
    }
    let env = Env::new();
    let root = env.root();
    let local = root.join("local");
    fs::create_dir_all(&local).unwrap();
    let mut denied = DeniedDirectories(Vec::new());
    for index in 0..105 {
        let path = root.join(format!("denied-{index:03}"));
        fs::create_dir(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();
        denied.0.push(path);
    }
    for backend in ["v1", "v2"] {
        let scan = answer(
            &env,
            &[
                "--backend",
                backend,
                "scan",
                root.to_str().unwrap(),
                "--json",
            ],
        );
        assert_eq!(scan["unknown_subtrees"].as_array().unwrap().len(), 105);
        assert_eq!(scan["errors"], 105);
        let status = answer(&env, &["--backend", backend, "status", "--json"]);
        assert_eq!(status["unknown_subtrees"].as_array().unwrap().len(), 100);
        let refresh = answer(
            &env,
            &[
                "--backend",
                backend,
                "refresh",
                local.to_str().unwrap(),
                "--json",
            ],
        );
        assert_eq!(refresh["unknown_subtrees"].as_array().unwrap().len(), 105);
        assert_eq!(scan["unknown_subtrees"], refresh["unknown_subtrees"]);
        let disk = answer(
            &env,
            &[
                "--backend",
                backend,
                "scan",
                root.to_str().unwrap(),
                "--low-memory",
                "--json",
            ],
        );
        assert_eq!(disk["unknown_subtrees"], scan["unknown_subtrees"]);
        let status = answer(&env, &["--backend", backend, "status", "--json"]);
        assert!(
            status["warnings"][0]
                .as_str()
                .unwrap()
                .starts_with("105 directories")
        );
    }
}

#[test]
fn v2_scan_honors_config_and_cli_memory_profiles_and_previous_policy() {
    let env = Env::new();
    fs::create_dir_all(env.root()).unwrap();
    fs::write(env.root().join("file"), b"first").unwrap();
    fs::write(
        env.config(),
        "backend = \"v2\"\nlow_memory = true\nkeep_previous = false\n",
    )
    .unwrap();
    for generation in [1, 2] {
        let scan = answer(&env, &["scan", env.root().to_str().unwrap(), "--json"]);
        assert_eq!(scan["generation"], generation);
        assert_eq!(scan["prev_generation"], Value::Null);
        assert_eq!(scan["entries"], 2);
        assert_eq!(scan["snapshot_bytes"], v2_stored_bytes(&env));
    }
    fs::write(env.config(), "backend = \"v2\"\n").unwrap();
    let scan = answer(
        &env,
        &[
            "scan",
            env.root().to_str().unwrap(),
            "--low-memory",
            "--json",
        ],
    );
    assert_eq!(scan["generation"], 3);
    assert_eq!(scan["prev_generation"], 2);
    assert_eq!(scan["snapshot_bytes"], v2_stored_bytes(&env));
    assert!(!env.snap().join("snapshot.db").exists());
    let status = answer(&env, &["status", "--json"]);
    assert_eq!(status["generation"], 3);
    assert_eq!(status["prev_generation"], 2);
    assert_eq!(status["entries"], 2);
    let files: Vec<_> = fs::read_dir(env.snap().join("v2"))
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert!(
        !files
            .iter()
            .any(|name| name.to_string_lossy().ends_with(".stage.tmp"))
    );
    fs::write(env.root().join("new-file"), [1; 8192]).unwrap();
    let refresh = answer(&env, &["refresh", env.root().to_str().unwrap(), "--json"]);
    assert_eq!(refresh["snapshot_bytes"], v2_stored_bytes(&env));
}

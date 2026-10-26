mod common;
use common::Env;
use serde_json::Value;

fn answer(e: &Env, args: &[&str]) -> Value {
    let output = e.run(args);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn explicit_backend_isolated_scan_refresh_history_and_failure() {
    let e = Env::new();
    let root = e.root();
    std::fs::create_dir_all(root.join("sub")).unwrap();
    std::fs::write(root.join("sub/a"), b"first").unwrap();
    let path = root.to_str().unwrap();
    let sub = root.join("sub");
    let first = answer(&e, &["--backend", "v2", "scan", path, "--json"]);
    assert_eq!(first["generation"], 1);
    assert_eq!(first["prev_generation"], Value::Null);
    assert!(e.snap().join("v2/generations.db").exists());
    assert!(!e.snap().join("snapshot.db").exists());
    assert!(!e.run(&["status", "--json"]).status.success());
    let second = answer(&e, &["scan", path, "--backend", "v2", "--json"]);
    assert_eq!(second["generation"], 2);
    assert_eq!(second["prev_generation"], 1);
    std::fs::write(sub.join("b"), vec![2u8; 8192]).unwrap();
    let refresh = answer(
        &e,
        &[
            "--backend",
            "v2",
            "refresh",
            sub.to_str().unwrap(),
            "--json",
        ],
    );
    assert_eq!(refresh["generation"], 3);
    let status = answer(&e, &["--backend", "v2", "status", "--json"]);
    assert_eq!(status["prev_generation"], 1);
    assert_eq!(status["refreshed"].as_array().unwrap().len(), 1);
    for command in ["ls", "top", "find", "diff"] {
        let value = answer(&e, &["--backend", "v2", command, path, "--json"]);
        assert_eq!(value["generation"], 3, "{command}: {value}");
    }
    let bad = e.run(&[
        "--backend",
        "v2",
        "scan",
        "/does/not/exist/freespace",
        "--json",
    ]);
    assert!(!bad.status.success());
    let status = answer(&e, &["--backend", "v2", "status", "--json"]);
    assert_eq!(status["generation"], 3);
    assert_eq!(status["prev_generation"], 1);
    assert!(status["last_error"].is_object());
    answer(&e, &["scan", path, "--json"]);
    assert!(e.snap().join("snapshot.db").exists());
    assert_eq!(answer(&e, &["status", "--json"])["generation"], 1);
    assert_eq!(
        answer(&e, &["--backend", "v2", "status", "--json"])["generation"],
        3
    );
}

#[test]
fn config_opt_in_preflight_and_foreign_database() {
    let e = Env::new();
    std::fs::create_dir_all(e.root()).unwrap();
    std::fs::write(e.config(), "backend = \"v2\"\nlow_memory = true\n").unwrap();
    let result = e.run(&["scan", e.root().to_str().unwrap(), "--json"]);
    assert!(!result.status.success());
    assert!(!e.snap().exists());
    std::fs::write(e.config(), "backend = \"v2\"\nkeep_previous = false\n").unwrap();
    assert!(
        !e.run(&["scan", e.root().to_str().unwrap(), "--json"])
            .status
            .success()
    );
    assert!(!e.snap().exists());
    std::fs::write(e.config(), "backend = \"v2\"\n").unwrap();
    assert!(!e.run(&["ls", "--json"]).status.success());
    assert!(!e.snap().exists());
    std::fs::create_dir_all(e.snap().join("v2")).unwrap();
    let db = e.snap().join("v2/generations.db");
    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.execute_batch(
        "CREATE TABLE foreign_data(value TEXT); INSERT INTO foreign_data VALUES('keep');",
    )
    .unwrap();
    for args in [
        vec!["status", "--json"],
        vec!["ls", "--json"],
        vec!["scan", e.root().to_str().unwrap(), "--json"],
    ] {
        let result = e.run(&args);
        assert!(!result.status.success());
    }
    assert_eq!(
        conn.query_row("SELECT value FROM foreign_data", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "keep"
    );
    answer(
        &e,
        &[
            "--backend",
            "v1",
            "scan",
            e.root().to_str().unwrap(),
            "--json",
        ],
    );
}

#[test]
fn v2_status_text_and_json_agree_on_growth_and_generation() {
    let e = Env::new();
    let root = e.root();
    let sub = root.join("sub");
    std::fs::create_dir_all(&sub).unwrap();
    std::fs::write(sub.join("a"), vec![1u8; 4096]).unwrap();
    for backend in ["v1", "v2"] {
        answer(
            &e,
            &[
                "--backend",
                backend,
                "scan",
                root.to_str().unwrap(),
                "--json",
            ],
        );
    }
    std::fs::write(sub.join("b"), vec![1u8; 65536]).unwrap();
    for backend in ["v1", "v2"] {
        let scan = e.run(&[
            "--backend",
            backend,
            "scan",
            root.to_str().unwrap(),
            "--json",
        ]);
        assert!(scan.status.success());
        let report: Value = serde_json::from_slice(&scan.stdout).unwrap();
        assert_eq!(report["generation"], 2);
        assert!(String::from_utf8_lossy(&scan.stderr).contains("generation 2 published"));
        if backend == "v2" {
            assert!(String::from_utf8_lossy(&scan.stderr).contains("walk includes finalization"));
            assert_eq!(report["finalize_seconds"], 0.0);
            assert_eq!(report["publish_seconds"], 0.0);
        }
    }
    let v1 = e.run(&["status"]);
    let v2 = e.run(&["--backend", "v2", "status"]);
    let v1 = String::from_utf8(v1.stdout).unwrap();
    let v2 = String::from_utf8(v2.stdout).unwrap();
    let hint = format!("since prev in {}", sub.display());
    assert!(v1.contains(&hint), "{v1}");
    assert!(v2.contains(&hint), "{v2}");
    assert!(v2.contains("generation 2"));
    let status = answer(&e, &["--backend", "v2", "status", "--json"]);
    assert_eq!(status["generation"], 2);
    assert_eq!(status["prev_generation"], 1);
}

#[test]
fn v2_install_lifecycle_refuses_before_mutating_files() {
    use std::os::unix::fs::PermissionsExt;
    let e = Env::new();
    let expected_error = if unsafe { libc::geteuid() } == 0 {
        "do not yet support the v2 backend"
    } else {
        "NOT_ROOT"
    };
    let units = e.tmp.path().join("units");
    let bin = e.tmp.path().join("bin");
    let called = e.tmp.path().join("systemctl-called");
    std::fs::create_dir(&units).unwrap();
    std::fs::create_dir(&bin).unwrap();
    let unit = units.join("freespace.service");
    std::fs::write(&unit, "preserve existing service").unwrap();
    let systemctl = bin.join("systemctl");
    std::fs::write(
        &systemctl,
        "#!/bin/sh\nprintf called > \"$FREESPACE_TEST_SYSTEMCTL\"\nexit 0\n",
    )
    .unwrap();
    std::fs::set_permissions(&systemctl, std::fs::Permissions::from_mode(0o755)).unwrap();
    let run = |args: &[&str]| {
        e.cmd(args)
            .env("FREESPACE_UNIT_DIR", &units)
            .env("FREESPACE_TEST_SYSTEMCTL", &called)
            .env("PATH", &bin)
            .output()
            .unwrap()
    };
    std::fs::create_dir_all(e.snap().join("v2")).unwrap();
    let data = e.snap().join("v2/generations.db");
    std::fs::write(&data, b"preserve v2 data").unwrap();
    for config in ["", "backend = \"v2\"\n"] {
        std::fs::write(e.config(), config).unwrap();
        for uninstall in [false, true] {
            let mut args = vec!["--backend", "v2", "install-timer", "--json"];
            if uninstall {
                args.push("--uninstall");
            }
            let output = run(&args);
            assert!(!output.status.success());
            assert!(String::from_utf8_lossy(&output.stdout).contains(expected_error));
            assert_eq!(std::fs::read_to_string(e.config()).unwrap(), config);
            assert_eq!(std::fs::read(&data).unwrap(), b"preserve v2 data");
        }
    }
    // Overriding the current invocation cannot silently rewrite a persisted
    // v2 configuration through the v1 installer lifecycle either.
    let output = run(&["--backend", "v1", "install-timer", "--uninstall", "--json"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains(expected_error));
    assert_eq!(std::fs::read(&data).unwrap(), b"preserve v2 data");
    assert_eq!(
        std::fs::read_to_string(&unit).unwrap(),
        "preserve existing service"
    );
    assert_eq!(std::fs::read_dir(&units).unwrap().count(), 1);
    assert!(!called.exists(), "refusal must not invoke systemctl");
}

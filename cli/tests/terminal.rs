//! Terminal data is escaped without changing JSON values or raw path selection.
use std::ffi::OsStr;
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::process::CommandExt;

use freespace_core::snapshot::SnapshotDir;
use freespace_core::snapshot::state::{self, LastError};
mod common;
use common::Env;

const NAME: &str = "имя\u{1b}\n\u{85}\u{7f}\t";
const DISPLAY: &str = r"имя\u{1b}\n\u{85}\u{7f}\t";

fn safe(bytes: &[u8]) -> String {
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(
        text.chars().all(|c| !c.is_control() || c == '\n'),
        "{text:?}"
    );
    text
}

fn scan(e: &Env) {
    let o = e.run(&[
        "scan",
        e.root().to_str().unwrap(),
        "--workers",
        "1",
        "--json",
    ]);
    assert!(o.status.success(), "{:?}", o.stderr);
    safe(&o.stderr);
}

#[test]
fn terminal_look_commands_escape_data_and_preserve_json_paths() {
    let e = Env::new();
    fs::create_dir(e.root()).unwrap();
    scan(&e);
    let folder = e.root().join(NAME);
    fs::create_dir(&folder).unwrap();
    fs::write(folder.join(NAME), b"content").unwrap();
    let raw = folder.join(OsStr::from_bytes(b"raw\xff\x1b"));
    fs::write(&raw, b"raw").unwrap();
    scan(&e);
    let path = folder.to_str().unwrap();
    for args in [
        vec!["ls", path],
        vec!["top", path],
        vec!["find", path],
        vec!["diff", path, "--kind", "any"],
    ] {
        let o = e.run(&args);
        assert!(o.status.success(), "{args:?}: {:?}", o.stderr);
        let text = safe(&o.stdout);
        assert!(text.lines().next().unwrap().contains(DISPLAY), "{text:?}");
        assert!(
            text.contains(&format!("{DISPLAY}/{DISPLAY}"))
                || text.lines().skip(2).any(|line| line.ends_with(DISPLAY)),
            "{text:?}"
        );
        assert!(
            text.contains("raw�\\u{1b}  [name is not UTF-8, see --json path_raw]"),
            "{text:?}"
        );
        let mut json_args = args.clone();
        json_args.push("--json");
        let json = e.run(&json_args);
        assert!(json.stdout.windows(2).any(|w| w == [0xc2, 0x85]));
        let v: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
        assert_eq!(
            v.get("path").or_else(|| v["entry"].get("path")).unwrap(),
            path
        );
        let entries = v["entries"].as_array().unwrap();
        let raw_entry = entries
            .iter()
            .map(|x| x.get("entry").unwrap_or(x))
            .find(|x| x["path_raw"].is_string())
            .unwrap();
        let selected = e.run(&[
            "ls",
            "--path-raw",
            raw_entry["path_raw"].as_str().unwrap(),
            "--json",
        ]);
        let selected: serde_json::Value = serde_json::from_slice(&selected.stdout).unwrap();
        assert_eq!(selected["entry"]["path_raw"], raw_entry["path_raw"]);
    }
    let o = e.run(&["refresh", path, "--json"]);
    assert!(o.status.success(), "{:?}", o.stderr);
    assert!(safe(&o.stderr).contains(DISPLAY));
}

#[test]
fn terminal_diagnostics_escape_data_without_changing_json() {
    let e = Env::new();
    fs::create_dir_all(e.root().join(NAME)).unwrap();
    scan(&e);
    let dir = SnapshotDir::new(e.snap());
    let mut st = state::read(&dir).unwrap().unwrap();
    st.last_error = Some(LastError {
        ts: st.last_success_ts.unwrap() + 60,
        op: NAME.into(),
        message: NAME.into(),
    });
    st.warnings = vec![NAME.into()];
    state::write(&dir, &st).unwrap();
    for command in ["status", "doctor"] {
        let o = e.run(&[command]);
        assert!(safe(&o.stdout).contains(DISPLAY));
        let o = e.run(&[command, "--json"]);
        let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
        if command == "status" {
            assert_eq!(v["last_error"]["message"], NAME);
            assert_eq!(v["warnings"][0], NAME);
        } else {
            assert!(
                v["checks"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|c| c["detail"].as_str().unwrap().contains(NAME))
            );
        }
    }
    let absent = e.root().join(NAME).join("absent");
    let o = e.run(&["ls", absent.to_str().unwrap(), "--json"]);
    assert_eq!(o.status.code(), Some(2));
    let err = safe(&o.stderr);
    assert_eq!(err.lines().count(), 2);
    assert!(err.lines().all(|line| line.contains(DISPLAY)));
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert!(v.to_string().contains("\\u001b"));
    assert!(v["error"]["message"].as_str().unwrap().contains(NAME));
}

#[test]
fn terminal_unknown_subtrees_and_argument_errors_escape_data() {
    let e = Env::new();
    fs::create_dir_all(e.root().join(NAME)).unwrap();
    scan(&e);
    // Mark the known directory unreadable to exercise the persisted data boundary
    // without requiring different filesystem permissions from the test runner.
    let db = rusqlite::Connection::open(e.snap().join("snapshot.db")).unwrap();
    db.execute(
        "UPDATE entries SET flags = flags | ?1 WHERE name = ?2",
        rusqlite::params![
            i64::from(freespace_core::scan::flags::DENIED),
            NAME.as_bytes()
        ],
    )
    .unwrap();
    db.execute(
        "INSERT INTO denied (entry_id) SELECT id FROM entries WHERE name = ?1",
        [NAME.as_bytes()],
    )
    .unwrap();
    drop(db);
    let o = e.run(&["ls", e.root().to_str().unwrap()]);
    assert!(safe(&o.stderr).contains(DISPLAY));
    assert!(safe(&e.run(&["status"]).stdout).contains(&format!(
        "unknown: {}/{}",
        e.root().display(),
        DISPLAY
    )));
    for args in [
        vec!["ls", "--sort", NAME, "--json"],
        vec![NAME, "--json"],
        vec!["ls", "--имя\u{1b}\n\u{85}\u{7f}\t", "--json"],
    ] {
        let o = e.run(&args);
        assert_eq!(o.status.code(), Some(2));
        let stderr = safe(&o.stderr);
        assert!(stderr.contains(DISPLAY));
        assert!(!stderr.contains("tip:"));
        let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
        // The existing first-line machine diagnostic keeps its original value.
        assert!(v["error"]["message"].as_str().unwrap().contains("имя"));
    }
    let c1 = e.run(&["ls", "--sort", "имя\u{85}", "--json"]);
    assert!(safe(&c1.stderr).contains(r"имя\u{85}"));
    let v: serde_json::Value = serde_json::from_slice(&c1.stdout).unwrap();
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap()
            .contains("имя\u{85}")
    );
    let ordinary = e.run(&["ls", "--ordinary"]);
    assert_eq!(
        String::from_utf8(ordinary.stderr).unwrap(),
        "error: unexpected argument '--ordinary' found\n\n  tip: to pass '--ordinary' as a value, use '-- --ordinary'\n\nUsage: freespace ls [OPTIONS] [PATH]\n\nFor more information, try '--help'.\n"
    );
}

#[test]
fn terminal_scan_bare_and_skill_paths_escape_data() {
    let e = Env::new();
    let root = e.root().join(NAME);
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("обычный"), b"data").unwrap();
    let path = root.to_str().unwrap();
    let o = e.run(&["scan", path, "--workers", "1", "--json"]);
    assert!(o.status.success(), "{:?}", o.stderr);
    assert!(safe(&o.stderr).contains(DISPLAY));
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["roots"][0], path);
    let o = e.run(&[]);
    assert!(o.status.success());
    let text = safe(&o.stdout);
    assert!(text.contains("Next:\n"));
    for line in text
        .lines()
        .filter_map(|line| line.strip_prefix("  freespace "))
    {
        let command = line.split("  ").next().unwrap();
        let parsed = std::process::Command::new("sh")
            .args(["-c", &format!("set -- {command}; printf '%s\\0' \"$@\"")])
            .output()
            .unwrap();
        assert!(parsed.status.success(), "{parsed:?}");
        let args: Vec<_> = parsed
            .stdout
            .split(|b| *b == 0)
            .filter(|arg| !arg.is_empty())
            .map(OsStr::from_bytes)
            .collect();
        let o = e.cmd(&[]).args(&args).output().unwrap();
        assert!(o.status.success(), "{args:?}: {:?}", o.stderr);
        let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
        assert_eq!(
            v.get("path").or_else(|| v["entry"].get("path")).unwrap(),
            path
        );
    }
    let skill = e.tmp.path().join(NAME);
    for command in ["install", "uninstall"] {
        let o = e.run(&["skill", command, "--dir", skill.to_str().unwrap()]);
        assert!(o.status.success(), "{:?}", o.stderr);
        assert!(safe(&o.stderr).contains(DISPLAY));
    }
}

#[test]
fn terminal_clap_usage_escapes_the_executable_name() {
    let e = Env::new();
    for args in [
        vec!["ls", "--ordinary"],
        vec!["--help"],
        vec!["ls", "--help"],
    ] {
        let o = e.cmd(&args).arg0(NAME).output().unwrap();
        let text = safe(&[o.stdout, o.stderr].concat());
        assert!(text.contains(&format!("Usage: {DISPLAY}")), "{text:?}");
    }
    let ordinary = e
        .cmd(&["ls", "--ordinary"])
        .arg0("renamed-freespace")
        .output()
        .unwrap();
    assert!(safe(&ordinary.stderr).contains("Usage: renamed-freespace ls "));
    let raw_name = OsStr::from_bytes(b"raw\xff\rname");
    let raw = e
        .cmd(&["ls", "--ordinary"])
        .arg0(raw_name)
        .output()
        .unwrap();
    let text = safe(&raw.stderr);
    assert!(text.contains(r"\rname"), "{text:?}");
}

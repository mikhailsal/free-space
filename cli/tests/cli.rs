//! The binary end to end on a temp snapshot dir: exit codes, the D16 error body
//! under `--json`, tables, bare `freespace`, relative paths, refresh.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

struct Env {
    tmp: tempfile::TempDir,
}

impl Env {
    fn new() -> Self {
        Env {
            tmp: tempfile::tempdir().unwrap(),
        }
    }

    fn root(&self) -> PathBuf {
        self.tmp.path().join("r")
    }

    fn cmd(&self, args: &[&str]) -> Command {
        let mut c = Command::new(env!("CARGO_BIN_EXE_freespace"));
        c.args(args)
            .env("FREESPACE_CONFIG", self.tmp.path().join("none.toml"))
            .env("FREESPACE_SNAPSHOT_DIR", self.tmp.path().join("snap"));
        c
    }

    fn run(&self, args: &[&str]) -> Output {
        self.cmd(args).output().unwrap()
    }

    fn fixture(&self) {
        let r = self.root();
        fs::create_dir_all(r.join("sub/deep")).unwrap();
        fs::write(r.join("sub/deep/disk.bin"), vec![1u8; 300 << 10]).unwrap();
        fs::write(r.join("sub/notes.txt"), vec![1u8; 20 << 10]).unwrap();
        fs::write(r.join("top.bin"), vec![1u8; 100 << 10]).unwrap();
    }

    fn scan(&self) {
        let o = self.run(&["scan", s(&self.root()), "--json"]);
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    }
}

fn s(p: &Path) -> &str {
    p.to_str().unwrap()
}

fn json(o: &Output) -> Value {
    serde_json::from_slice(&o.stdout)
        .unwrap_or_else(|e| panic!("{e}: {}", String::from_utf8_lossy(&o.stdout)))
}

fn text(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn code(o: &Output) -> i32 {
    o.status.code().unwrap()
}

#[test]
fn help_is_grouped_with_examples() {
    let e = Env::new();
    let o = e.run(&["--help"]);
    assert_eq!(code(&o), 0);
    let h = text(&o);
    for g in ["Look:", "Maintain:", "Setup:", "ls ", "diff ", "refresh "] {
        assert!(h.contains(g), "{g} missing in\n{h}");
    }
    for c in [
        "status", "ls", "top", "find", "diff", "scan", "refresh", "schema",
    ] {
        let o = e.run(&[c, "--help"]);
        assert_eq!(code(&o), 0, "{c}");
        assert!(text(&o).contains("Examples:"), "{c} --help has no examples");
    }
    assert_eq!(code(&e.run(&["--version"])), 0);
}

#[test]
fn no_snapshot_is_exit_8_with_a_json_body() {
    let e = Env::new();
    let o = e.run(&["ls", "/", "--json"]);
    assert_eq!(code(&o), 8);
    let v = json(&o);
    assert_eq!(v["error"]["code"], "NO_SNAPSHOT");
    assert_eq!(v["error"]["exit"], 8);
    assert!(v["error"]["fix"].as_str().unwrap().contains("install.sh"));
    assert!(
        v["error"]["context"]["snapshot_dir"]
            .as_str()
            .unwrap()
            .ends_with("snap")
    );
    assert!(String::from_utf8_lossy(&o.stderr).starts_with("freespace: no snapshot"));

    // without --json: text on stderr only
    let o = e.run(&["ls", "/"]);
    assert_eq!(code(&o), 8);
    assert!(o.stdout.is_empty());

    // bare freespace: install command, exit 0 (D10)
    let o = e.run(&[]);
    assert_eq!(code(&o), 0);
    let t = text(&o);
    assert!(t.starts_with("No snapshot yet in"), "{t}");
    assert!(t.contains(freespace_core::INSTALL_CMD));
}

#[test]
fn usage_errors_are_exit_2_and_json_under_json() {
    let e = Env::new();
    let o = e.run(&["ls", "--bogus", "--json"]);
    assert_eq!(code(&o), 2);
    assert_eq!(json(&o)["error"]["code"], "USAGE");
    let o = e.run(&["top", "--frobnicate"]);
    assert_eq!(code(&o), 2);
    assert!(o.stdout.is_empty());
}

#[test]
fn scan_then_look_commands() {
    let e = Env::new();
    e.fixture();
    e.scan();
    let root = e.root();

    let o = e.run(&["ls", s(&root)]);
    assert_eq!(code(&o), 0);
    let t = text(&o);
    let lines: Vec<&str> = t.lines().collect();
    assert!(lines[0].starts_with(s(&root)), "{t}");
    assert!(lines[1].starts_with("ALLOCATED"), "{t}");
    assert!(lines[2].ends_with("sub/"), "biggest first: {t}");
    assert!(!t.contains('\x1b'), "no ANSI in tables");

    let v = json(&e.run(&["ls", s(&root), "--json"]));
    assert_eq!(v["entries"][0]["name"], "sub");
    assert_eq!(v["generation"], 1);
    assert_eq!(v["incomplete"], false);

    let v = json(&e.run(&["top", s(&root), "--json", "--limit", "1"]));
    assert_eq!(v["entries"][0]["name"], "disk.bin");
    assert_eq!(v["truncated"], true);
    assert_eq!(v["total"]["count"], 3);

    let v = json(&e.run(&["find", s(&root), "--name", "*.bin", "--json"]));
    assert_eq!(v["total"]["count"], 2);

    let o = e.run(&["diff", s(&root), "--json"]);
    assert_eq!(code(&o), 3);
    assert_eq!(json(&o)["error"]["code"], "NO_PREV");

    let v = json(&e.run(&["status", "--json"]));
    assert_eq!(v["generation"], 1);
    assert_eq!(v["roots"][0]["path"], s(&root));

    // piped bare freespace: sentence, then Next lines with --json (D15)
    let o = e.run(&[]);
    assert_eq!(code(&o), 0);
    let t = text(&o);
    assert!(t.lines().next().unwrap().contains("snapshot"), "{t}");
    let next = format!("Next:\n  freespace ls {} --json", s(&root));
    assert!(t.contains(&next), "{t}");
    assert_eq!(json(&e.run(&["--json"]))["generation"], 1);

    let schema = e.run(&["schema", "--json"]);
    let want =
        fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("../skill/schema.json")).unwrap();
    assert_eq!(schema.stdout, want);
}

#[test]
fn relative_paths_start_at_the_current_directory() {
    let e = Env::new();
    e.fixture();
    e.scan();
    let o = e
        .cmd(&["ls", "deep", "--json"])
        .current_dir(e.root().join("sub"))
        .output()
        .unwrap();
    assert_eq!(code(&o), 0, "{}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(json(&o)["entry"]["path"], s(&e.root().join("sub/deep")));

    let o = e.run(&["ls", s(&e.root().join("missing/x")), "--json"]);
    assert_eq!(code(&o), 2);
    let v = json(&o);
    assert_eq!(v["error"]["code"], "PATH_NOT_FOUND");
}

#[test]
fn refresh_republishes_and_diff_sees_changes() {
    let e = Env::new();
    e.fixture();
    e.scan();
    fs::write(e.root().join("sub/new.bin"), vec![1u8; 200 << 10]).unwrap();
    let o = e.run(&["refresh", s(&e.root().join("sub")), "--json"]);
    assert_eq!(code(&o), 0, "{}", String::from_utf8_lossy(&o.stderr));
    let v = json(&o);
    assert_eq!(v["generation"], 2);
    assert_eq!(v["removed"], false);
    let v = json(&e.run(&["status", "--json"]));
    assert_eq!(v["refreshed"][0]["path"], s(&e.root().join("sub")));

    // a second full scan gives diff a previous generation
    e.scan();
    let o = e.run(&["diff", s(&e.root())]);
    assert_eq!(code(&o), 0, "{}", String::from_utf8_lossy(&o.stderr));
    let t = text(&o);
    assert!(t.contains("since"), "{t}");
}

#[test]
fn install_timer_needs_root() {
    if euid_is_root() {
        return;
    }
    let e = Env::new();
    let o = e.run(&["install-timer", "--json"]);
    assert_eq!(code(&o), 1);
    assert_eq!(json(&o)["error"]["code"], "NOT_ROOT");
    assert!(
        json(&o)["error"]["fix"]
            .as_str()
            .unwrap()
            .starts_with("sudo freespace install-timer")
    );
}

fn euid_is_root() -> bool {
    Command::new("id")
        .arg("-u")
        .output()
        .is_ok_and(|o| o.stdout.starts_with(b"0\n"))
}

#[test]
fn doctor_reports_checks_with_fixes() {
    let e = Env::new();
    e.fixture();
    e.scan();
    let o = e.run(&["doctor", "--json"]);
    let v = json(&o);
    let checks = v["checks"].as_array().unwrap();
    let ids: Vec<&str> = checks.iter().map(|c| c["id"].as_str().unwrap()).collect();
    for id in [
        "config",
        "timer",
        "binary",
        "path",
        "unit",
        "access",
        "freshness",
        "last_error",
        "space",
        "skill",
    ] {
        assert!(ids.contains(&id), "{id} missing from {ids:?}");
    }
    for c in checks {
        assert_eq!(c["ok"].as_bool().unwrap(), c["fix"].is_null(), "{c}");
    }
    let access = checks.iter().find(|c| c["id"] == "access").unwrap();
    assert_eq!(access["ok"], true);
    assert_eq!(code(&o), if v["ok"] == true { 0 } else { 1 });
}

#[test]
fn skill_install_writes_stamped_files() {
    let e = Env::new();
    let dir = e.tmp.path().join("skills/freespace");
    let o = e.run(&["skill", "install", "--dir", s(&dir)]);
    assert_eq!(code(&o), 0, "{}", String::from_utf8_lossy(&o.stderr));
    let md = fs::read_to_string(dir.join("SKILL.md")).unwrap();
    assert!(md.starts_with("---\nname: freespace\n"));
    assert!(md.contains(&format!(
        "<!-- freespace-version: {} -->",
        env!("CARGO_PKG_VERSION")
    )));
    let printed = text(&e.run(&["skill", "print"]));
    assert_eq!(printed, md);
    let schema = fs::read(dir.join("schema.json")).unwrap();
    let want =
        fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("../skill/schema.json")).unwrap();
    assert_eq!(schema, want);
    assert_eq!(code(&e.run(&["skill", "uninstall", "--dir", s(&dir)])), 0);
    assert!(!dir.exists());
}

#[test]
fn completions_for_three_shells() {
    let e = Env::new();
    for sh in ["bash", "zsh", "fish"] {
        let o = e.run(&["completions", sh]);
        assert_eq!(code(&o), 0);
        let t = text(&o);
        assert!(t.contains("install-timer"), "{sh}");
        assert!(t.contains("any"), "{sh}: --kind values");
    }
    let bash = e.run(&["completions", "bash"]).stdout;
    let mut check = Command::new("bash")
        .arg("-n")
        .stdin(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    use std::io::Write;
    check.stdin.take().unwrap().write_all(&bash).unwrap();
    assert!(check.wait().unwrap().success());
}

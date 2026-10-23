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

/// Reads a child's output lines on a thread so a test can wait for one.
fn lines_of(r: impl std::io::Read + Send + 'static) -> std::sync::mpsc::Receiver<String> {
    use std::io::BufRead;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for l in std::io::BufReader::new(r).lines().map_while(|l| l.ok()) {
            if tx.send(l).is_err() {
                break;
            }
        }
    });
    rx
}

fn wait_for(rx: &std::sync::mpsc::Receiver<String>, what: &str) -> String {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    let mut seen = String::new();
    while let Some(left) = deadline.checked_duration_since(std::time::Instant::now()) {
        match rx.recv_timeout(left) {
            Ok(l) if l.contains(what) => return l,
            Ok(l) => seen.push_str(&format!("{l}\n")),
            Err(_) => break,
        }
    }
    panic!("no line with {what:?}; got:\n{seen}");
}

/// Token and port from a `http://127.0.0.1:P/#...t=TOKEN` line.
fn url_parts(line: &str) -> (u16, String) {
    let at = line.find("http://127.0.0.1:").unwrap() + "http://127.0.0.1:".len();
    let port = line[at..].split('/').next().unwrap().parse().unwrap();
    let t = line.rsplit("t=").next().unwrap();
    let token: String = t.chars().take_while(|c| c.is_ascii_hexdigit()).collect();
    assert_eq!(token.len(), 64, "{line}");
    (port, token)
}

fn http_get(port: u16, path: &str, token: &str) -> String {
    use std::io::{Read, Write};
    let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(
        s,
        "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nX-Freespace-Token: {token}\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).unwrap();
    out
}

fn signal(child: &std::process::Child, sig: i32) {
    // SAFETY: plain kill(2) on our own child's pid
    unsafe { libc::kill(child.id() as i32, sig) };
}

/// Stop the freespace that `script` runs (not `script` itself: closing the
/// pty under a live server leaves it blocked on the terminal), then reap both.
fn stop_under_tty(mut child: std::process::Child) {
    let list = format!("/proc/{0}/task/{0}/children", child.id());
    let inner: Vec<i32> = fs::read_to_string(list)
        .unwrap_or_default()
        .split_whitespace()
        .filter_map(|p| p.parse().ok())
        .collect();
    for pid in inner {
        // SAFETY: kill(2) on the server our `script` child started
        unsafe { libc::kill(pid, libc::SIGTERM) };
    }
    let st = child.wait().unwrap();
    assert!(st.success(), "gui stops cleanly on SIGTERM: {st}");
}

#[test]
fn gui_without_a_terminal_refuses_with_gui_no_tty() {
    let e = Env::new();
    let o = e
        .cmd(&["gui"])
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    assert_eq!(code(&o), 2);
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(err.contains("gui needs a terminal"), "{err}");
    assert!(err.contains("freespace ls / --json"), "{err}");
}

#[test]
fn serve_prints_a_tokened_url_and_stops_cleanly() {
    let e = Env::new();
    e.fixture();
    e.scan();
    let mut child = e
        .cmd(&["serve", "--port", "0"])
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let rx = lines_of(child.stderr.take().unwrap());
    let line = wait_for(&rx, "freespace serve: http://127.0.0.1:");
    let (port, token) = url_parts(&line);
    let r = http_get(port, "/api/status", &token);
    assert!(r.starts_with("HTTP/1.1 200"), "{r}");
    assert!(r.contains("\"generation\":1"), "{r}");
    assert!(http_get(port, "/api/status", "nope").starts_with("HTTP/1.1 403"));
    signal(&child, libc::SIGTERM);
    assert!(
        child.wait().unwrap().success(),
        "SIGTERM is a clean stop (D8)"
    );
}

/// `script` gives `gui` the terminal it insists on; None when it is missing.
fn under_tty(e: &Env, args: &str, env: &[(&str, &str)]) -> Option<std::process::Child> {
    if Command::new("script").arg("--version").output().is_err() {
        eprintln!("skipped: no script(1) for a pseudo-terminal");
        return None;
    }
    let bin = env!("CARGO_BIN_EXE_freespace");
    let mut c = Command::new("script");
    c.args(["-qec", &format!("exec {bin} {args}"), "/dev/null"])
        .env_clear()
        .env("FREESPACE_CONFIG", e.tmp.path().join("none.toml"))
        .env("FREESPACE_SNAPSHOT_DIR", e.tmp.path().join("snap"))
        .env("HOME", e.tmp.path())
        .env("SHELL", "/bin/sh")
        .env("PATH", "/usr/bin:/bin");
    for (k, v) in env {
        c.env(k, v);
    }
    Some(
        c.stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap(),
    )
}

#[test]
fn gui_hands_the_token_through_a_private_file_it_then_removes() {
    let e = Env::new();
    e.fixture();
    e.scan();
    let run = e.tmp.path().join("run");
    let bin = e.tmp.path().join("bin");
    fs::create_dir_all(&run).unwrap();
    fs::create_dir_all(&bin).unwrap();
    let log = e.tmp.path().join("opened");
    // a stand-in browser: records what it was asked to open
    let fake = bin.join("xdg-open");
    fs::write(
        &fake,
        format!(
            "#!/bin/sh\nstat -c %a \"$1\" > {log}.mode\ncp \"$1\" {log}.html\necho \"$1\" > {log}.path\n",
            log = log.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&fake, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let path = format!("{}:/usr/bin:/bin", bin.display());
    let Some(mut child) = under_tty(
        &e,
        &format!("gui {} --port 0", s(&e.root().join("sub"))),
        &[
            ("XDG_RUNTIME_DIR", s(&run)),
            ("PATH", &path),
            ("DISPLAY", ":99"),
        ],
    ) else {
        return;
    };
    let rx = lines_of(child.stdout.take().unwrap());
    wait_for(&rx, "freespace gui on http://127.0.0.1:");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while !log.with_extension("path").exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let opened = fs::read_to_string(log.with_extension("path")).unwrap();
    let file = PathBuf::from(opened.trim());
    assert!(file.starts_with(&run), "{}", file.display());
    assert_eq!(
        fs::read_to_string(log.with_extension("mode"))
            .unwrap()
            .trim(),
        "600"
    );
    let html = fs::read_to_string(log.with_extension("html")).unwrap();
    let (port, token) = url_parts(&html);
    assert!(
        html.contains("/sub?t="),
        "the folder rides in the fragment: {html}"
    );
    assert!(file.exists(), "kept until the page calls the API");
    assert!(http_get(port, "/api/status", &token).starts_with("HTTP/1.1 200"));
    assert!(!file.exists(), "removed on the first authorized request");
    stop_under_tty(child);
}

#[test]
fn gui_over_ssh_prints_the_tunnel_instead_of_opening_a_browser() {
    let e = Env::new();
    e.fixture();
    e.scan();
    let Some(mut child) = under_tty(
        &e,
        "gui --port 0",
        &[("SSH_CONNECTION", "10.0.0.2 5000 10.0.0.1 22")],
    ) else {
        return;
    };
    let rx = lines_of(child.stdout.take().unwrap());
    let tunnel = wait_for(&rx, "ssh -N -L");
    let url = wait_for(&rx, "http://127.0.0.1:");
    let (port, _) = url_parts(&url);
    assert!(
        tunnel.contains(&format!("{port}:127.0.0.1:{port}")),
        "{tunnel}"
    );
    stop_under_tty(child);
}

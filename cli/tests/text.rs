//! The text answers, pinned before `cli/src/status.rs` and `cli/src/query.rs`
//! are split (D0 of docs/designs/code-quality-gates.md): the rows of `status`,
//! and the tables, notes and markers of `ls`, `top`, `find` and `diff`. Files
//! are whole 4 K blocks, so their sizes read the same on ext4 and tmpfs; the
//! sizes of folders, dates and ages are left out.

use std::ffi::OsStr;
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Command, Output};

use freespace_core::snapshot::SnapshotDir;
use freespace_core::snapshot::state::{self, LastError};

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

    fn snap(&self) -> PathBuf {
        self.tmp.path().join("snap")
    }

    /// A path under the root, as the CLI prints it ("" for the root).
    fn p(&self, rel: &str) -> String {
        let p = if rel.is_empty() {
            self.root()
        } else {
            self.root().join(rel)
        };
        p.to_str().unwrap().to_owned()
    }

    fn cmd(&self, args: &[&str]) -> Command {
        let mut c = Command::new(env!("CARGO_BIN_EXE_freespace"));
        c.args(args)
            .env("FREESPACE_CONFIG", self.tmp.path().join("none.toml"))
            .env("FREESPACE_SNAPSHOT_DIR", self.snap())
            .env_remove("FREESPACE_FAULT")
            .env_remove("FREESPACE_PAUSE");
        c
    }

    fn run(&self, args: &[&str]) -> Output {
        self.cmd(args).output().unwrap()
    }

    /// The stdout of a command that must succeed.
    fn ok(&self, args: &[&str]) -> String {
        let o = self.run(args);
        assert!(
            o.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&o.stderr)
        );
        String::from_utf8_lossy(&o.stdout).into_owned()
    }

    /// The look fixture: three files, a hard link to one of them, a symlink,
    /// an empty folder, a name that is not UTF-8 and `locked` folders.
    fn tree(&self, locked: usize) {
        let r = self.root();
        fs::create_dir_all(r.join("sub/deep")).unwrap();
        fs::create_dir(r.join("empty")).unwrap();
        fs::write(r.join("sub/deep/disk.bin"), vec![1u8; 300 << 10]).unwrap();
        fs::write(r.join("sub/notes.txt"), vec![1u8; 20 << 10]).unwrap();
        fs::write(r.join("top.bin"), vec![1u8; 100 << 10]).unwrap();
        fs::hard_link(r.join("top.bin"), r.join("sub/top-link.bin")).unwrap();
        std::os::unix::fs::symlink("deep", r.join("sub/link")).unwrap();
        fs::write(r.join(OsStr::from_bytes(b"bad\xffname")), b"").unwrap();
        for i in 1..=locked {
            fs::create_dir(r.join(format!("locked{i}"))).unwrap();
        }
    }

    /// A full scan of the root; the locked folders are unreadable only while
    /// it runs, so the temp dir can be removed afterwards.
    fn scan(&self, locked: usize) {
        let mode = |m| {
            for i in 1..=locked {
                let p = self.root().join(format!("locked{i}"));
                fs::set_permissions(p, fs::Permissions::from_mode(m)).unwrap();
            }
        };
        mode(0o000);
        let o = self.run(&["scan", &self.p("")]);
        mode(0o755);
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    }

    fn state_dir(&self) -> SnapshotDir {
        SnapshotDir::new(self.snap())
    }
}

/// Folders a normal user cannot read; none as root, who reads everything.
fn locked() -> usize {
    let root = Command::new("id")
        .arg("-u")
        .output()
        .is_ok_and(|o| o.stdout.starts_with(b"0\n"));
    if root { 0 } else { 6 }
}

/// The first line, then each `label  value` row after it.
fn sentence_and_rows(t: &str) -> (String, Vec<(String, String)>) {
    let mut lines = t.lines();
    let first = lines.next().unwrap_or_default().to_owned();
    let rows = lines
        .filter_map(|l| l.split_once("  "))
        .map(|(k, v)| (k.to_owned(), v.trim_start().to_owned()))
        .collect();
    (first, rows)
}

fn row<'a>(rows: &'a [(String, String)], label: &str) -> &'a str {
    rows.iter()
        .find(|(k, _)| k == label)
        .map(|(_, v)| v.as_str())
        .unwrap_or_else(|| panic!("no {label} row in {rows:?}"))
}

fn line_ending<'a>(t: &'a str, end: &str) -> &'a str {
    t.lines()
        .find(|l| l.ends_with(end))
        .unwrap_or_else(|| panic!("no line ends with {end:?} in\n{t}"))
}

/// "a, b, c, d, e and 1 more": the first five unreadable folders.
fn first_five(e: &Env, locked: usize) -> String {
    let five: Vec<String> = (1..=5).map(|i| e.p(&format!("locked{i}"))).collect();
    format!("{} and {} more", five.join(", "), locked - 5)
}

#[test]
fn status_prints_a_row_for_each_fact() {
    let e = Env::new();
    let n = locked();
    e.tree(n);
    e.scan(n);
    fs::write(e.root().join("new.bin"), vec![1u8; 52 << 10]).unwrap();
    fs::remove_file(e.root().join("sub/notes.txt")).unwrap();
    e.scan(n);
    e.ok(&["refresh", &e.p("sub")]);

    let t = e.ok(&["status"]);
    let (first, rows) = sentence_and_rows(&t);
    assert!(first.contains(" free · snapshot "), "{t}");
    assert!(
        first.contains(&format!(" old · +32 K since prev in {}", e.p(""))),
        "{t}"
    );
    let labels: Vec<&str> = rows.iter().map(|(k, _)| k.as_str()).collect();
    let mut want = vec![
        "host",
        "snapshot",
        "previous",
        "root",
        "refreshed",
        "last success",
    ];
    if n > 0 {
        want.extend(["warning", "incomplete"]);
    }
    assert_eq!(labels, want, "{t}");
    let host = row(&rows, "host");
    let version = format!(" · linux · freespace {}", env!("CARGO_PKG_VERSION"));
    assert!(host.ends_with(&version), "{host}");
    let snap = row(&rows, "snapshot");
    assert!(snap.starts_with("generation 3 · "), "{snap}");
    assert!(snap.contains(" entries · scanned "), "{snap}");
    assert!(
        snap.ends_with(&format!(" · {}", e.snap().display())),
        "{snap}"
    );
    assert!(row(&rows, "previous").starts_with("generation 1 · scanned "));
    let root = row(&rows, "root");
    assert!(root.starts_with(&format!("{} · ", e.p(""))), "{root}");
    for part in [" items · filesystem ", " used of "] {
        assert!(root.contains(part), "{part} in {root}");
    }
    assert!(root.ends_with(" free now"), "{root}");
    let refreshed = row(&rows, "refreshed");
    assert!(
        refreshed.starts_with(&format!("{} at ", e.p("sub"))),
        "{refreshed}"
    );
    assert!(refreshed.ends_with(" (generation 3)"), "{refreshed}");
    if n > 0 {
        assert!(
            first.ends_with(&format!(" · {n} unreadable folders")),
            "{t}"
        );
        assert_eq!(
            row(&rows, "warning"),
            format!(
                "{n} directories could not be read, sizes under them are unknown: {}",
                first_five(&e, n)
            )
        );
        assert_eq!(
            row(&rows, "incomplete"),
            format!("unreadable, sizes unknown: {}", first_five(&e, n))
        );
    }
    assert!(!t.contains("Run: freespace doctor"), "{t}");
}

#[test]
fn status_names_a_failed_run_and_an_unreadable_state_file() {
    let e = Env::new();
    e.tree(0);
    e.scan(0);
    let dir = e.state_dir();
    let mut st = state::read(&dir).unwrap().unwrap();
    st.last_error = Some(LastError {
        ts: st.last_success_ts.unwrap() + 60,
        op: "scan".into(),
        message: "scan failed: the disk went away".into(),
    });
    state::write(&dir, &st).unwrap();
    let t = e.ok(&["status"]);
    let (_, rows) = sentence_and_rows(&t);
    let le = row(&rows, "last error");
    assert!(
        le.ends_with(" scan: scan failed: the disk went away"),
        "{le}"
    );
    assert_eq!(t.lines().last(), Some("Run: freespace doctor"), "{t}");

    fs::write(e.snap().join("state.json"), "{nope").unwrap();
    let t = e.ok(&["status"]);
    let (_, rows) = sentence_and_rows(&t);
    let bad = row(&rows, "state.json");
    assert!(
        bad.starts_with("unreadable: state.json is corrupt: "),
        "{bad}"
    );
    assert!(rows.iter().all(|(k, _)| k != "last success"), "{t}");
}

/// A run held at the `prechecked` fault point (debug builds) until its pause
/// file is removed.
#[cfg(debug_assertions)]
fn held(e: &Env, args: &[&str]) -> std::process::Child {
    use std::time::{Duration, Instant};
    let mut child = e
        .cmd(args)
        .env("FREESPACE_PAUSE", "prechecked")
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let pause = e.snap().join(".pause-prechecked");
    let deadline = Instant::now() + Duration::from_secs(30);
    while !pause.exists() {
        assert!(child.try_wait().unwrap().is_none(), "{args:?} ended early");
        assert!(Instant::now() < deadline, "{args:?} never paused");
        std::thread::sleep(Duration::from_millis(20));
    }
    child
}

#[cfg(debug_assertions)]
fn release(e: &Env, mut child: std::process::Child) {
    fs::remove_file(e.snap().join(".pause-prechecked")).unwrap();
    assert!(child.wait().unwrap().success());
}

#[cfg(debug_assertions)]
#[test]
fn status_while_a_scan_or_a_refresh_runs() {
    let e = Env::new();
    e.tree(0);
    let (root, sub) = (e.p(""), e.p("sub"));
    let child = held(&e, &["scan", &root]);
    let pid = child.id();
    let t = e.ok(&["status"]);
    let (first, rows) = sentence_and_rows(&t);
    assert!(first.starts_with(&format!("Scanning {root} since ")), "{t}");
    assert!(first.contains(" files · "), "progress in {t}");
    let snap = format!("none yet in {}", e.snap().display());
    assert_eq!(row(&rows, "snapshot"), snap);
    let running = row(&rows, "running");
    assert!(
        running.starts_with(&format!("scanning {root} since ")),
        "{running}"
    );
    assert!(running.ends_with(&format!(" · pid {pid}")), "{running}");
    let o = e.run(&["doctor", "--json"]);
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    let access = v["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == "access")
        .unwrap();
    assert_eq!(access["ok"], true);
    assert_eq!(
        access["detail"],
        "no snapshot yet: the first scan is running"
    );
    release(&e, child);

    let child = held(&e, &["refresh", &sub]);
    let pid = child.id();
    let t = e.ok(&["status"]);
    let (first, rows) = sentence_and_rows(&t);
    assert!(
        first.contains(&format!(" · refreshing {sub} since ")),
        "{t}"
    );
    let running = row(&rows, "running");
    assert!(
        running.starts_with(&format!("refreshing {sub} since ")),
        "{running}"
    );
    assert!(running.contains(" files · "), "progress in {running}");
    assert!(running.ends_with(&format!(" · pid {pid}")), "{running}");
    release(&e, child);
}

#[test]
fn ls_marks_each_kind_and_says_why_a_listing_is_empty() {
    let e = Env::new();
    let n = locked();
    e.tree(n);
    e.scan(n);
    let root = e.p("");
    let o = e.run(&["ls", &root]);
    let t = String::from_utf8_lossy(&o.stdout).into_owned();
    let lines: Vec<&str> = t.lines().collect();
    assert!(lines[0].starts_with(&format!("{root}  ")), "{t}");
    assert!(lines[0].contains(" items · snapshot "), "{t}");
    assert_eq!(lines[1], "ALLOCATED  SHARE  ITEMS  MODIFIED    NAME");
    assert!(lines[2].ends_with("  sub/"), "biggest first: {t}");
    line_ending(&t, "  empty/");
    line_ending(
        &t,
        "  bad\u{fffd}name  [name is not UTF-8, see --json path_raw]",
    );
    let alias = format!("  top.bin  [hardlink, counted at {root}/sub/top-link.bin]");
    assert!(line_ending(&t, &alias).starts_with("      0 B"), "{t}");
    let warn = String::from_utf8_lossy(&o.stderr);
    if n > 0 {
        line_ending(&t, "  locked1/  [unreadable]");
        let head = "warning: incomplete snapshot, sizes under these folders are unknown: ";
        assert_eq!(warn, format!("{head}{}\n", first_five(&e, n)));
    } else {
        assert!(warn.is_empty(), "{warn}");
    }

    let t = e.ok(&["ls", &e.p("sub")]);
    assert!(
        line_ending(&t, "  top-link.bin").starts_with("    100 K"),
        "{t}"
    );
    line_ending(&t, "  link@");

    let mut notes = vec![
        ("empty", "Nothing here in this snapshot.".to_owned()),
        (
            "top.bin",
            format!("{} is a file, not a folder.", e.p("top.bin")),
        ),
        (
            "sub/link",
            format!("{} is a symlink, not a folder.", e.p("sub/link")),
        ),
    ];
    if n > 0 {
        let why = "Contents unavailable: this folder could not be read as root.";
        notes.push(("locked1", why.to_owned()));
    }
    for (rel, note) in notes {
        let t = e.ok(&["ls", &e.p(rel)]);
        assert_eq!(t.lines().count(), 2, "{t}");
        assert_eq!(t.lines().last(), Some(note.as_str()), "{rel}");
    }

    let t = e.ok(&["ls", &root, "--limit", "2"]);
    let more = format!("… {} more (show them with --limit {})", n + 2, n + 4);
    assert_eq!(t.lines().count(), 5, "{t}");
    assert_eq!(t.lines().last(), Some(more.as_str()), "{t}");
}

#[test]
fn top_lists_files_or_folders_under_its_filters() {
    let e = Env::new();
    let n = locked();
    e.tree(n);
    e.scan(n);
    let root = e.p("");
    let t = e.ok(&["top", &root]);
    let lines: Vec<&str> = t.lines().collect();
    let head = format!("5 files under {root} · 420 K · snapshot ");
    assert!(lines[0].starts_with(&head), "{t}");
    assert_eq!(lines[1], "ALLOCATED  MODIFIED    PATH");
    assert!(lines[2].starts_with("    300 K  "), "{t}");
    assert!(
        lines[2].ends_with(&format!("  {root}/sub/deep/disk.bin")),
        "{t}"
    );
    line_ending(
        &t,
        &format!("  {root}/top.bin  [hardlink, counted at {root}/sub/top-link.bin]"),
    );

    let t = e.ok(&["top", &root, "--kind", "dir"]);
    let lines: Vec<&str> = t.lines().collect();
    assert!(
        lines[0].contains(&format!(" folders under {root} · ")),
        "{t}"
    );
    assert_eq!(lines[1], "ALLOCATED  ITEMS  NEWEST FILE  PATH");
    let empty = line_ending(&t, &format!("  {root}/empty/"));
    assert!(empty.contains("      0  unknown      "), "{t}");
    if n > 0 {
        line_ending(&t, &format!("  {root}/locked1/  [unreadable]"));
    }

    let t = e.ok(&["top", &root, "--min-size", "50K", "--older-than", "1d"]);
    let lines: Vec<&str> = t.lines().collect();
    assert!(
        lines[0].starts_with("0 files ≥ 50 K untouched since "),
        "{t}"
    );
    assert!(
        lines[0].contains(&format!(" under {root} · 0 B · snapshot ")),
        "{t}"
    );
    assert_eq!(lines[1..], ["Nothing matches."]);

    let t = e.ok(&["top", &root, "--limit", "1"]);
    assert_eq!(t.lines().count(), 4, "{t}");
    assert_eq!(
        t.lines().last(),
        Some("… 4 more (show them with --limit 5)")
    );
}

#[test]
fn find_counts_nested_matches_once() {
    let e = Env::new();
    e.tree(0);
    e.scan(0);
    let root = e.p("");
    let t = e.ok(&["find", &root, "--name", "*.bin"]);
    let lines: Vec<&str> = t.lines().collect();
    let head = format!("3 matches under {root} · 400 K · snapshot ");
    assert!(lines[0].starts_with(&head), "{t}");
    assert_eq!(lines[1], "ALLOCATED  MODIFIED    PATH");
    assert!(
        lines[2].ends_with(&format!("  {root}/sub/deep/disk.bin")),
        "{t}"
    );
    line_ending(
        &t,
        &format!("  {root}/top.bin  [hardlink, counted at {root}/sub/top-link.bin]"),
    );

    let t = e.ok(&["find", &root, "--name", "d*"]);
    let lines: Vec<&str> = t.lines().collect();
    assert!(
        lines[0].starts_with(&format!("2 matches under {root} · ")),
        "{t}"
    );
    assert!(
        lines[0].contains(" (1 inside other matches, counted once) · snapshot "),
        "{t}"
    );
    line_ending(&t, &format!("  {root}/sub/deep/"));
    line_ending(
        &t,
        &format!("  {root}/sub/deep/disk.bin  (in parent total)"),
    );

    let t = e.ok(&["find", &root, "--name", "nothing*"]);
    let lines: Vec<&str> = t.lines().collect();
    let head = format!("0 matches under {root} · 0 B · snapshot ");
    assert!(lines[0].starts_with(&head), "{t}");
    assert_eq!(lines[1..], [format!("No matches within {root}.")]);

    let t = e.ok(&["find", &root, "--name", "*.bin", "--limit", "1"]);
    assert_eq!(t.lines().count(), 4, "{t}");
    assert_eq!(
        t.lines().last(),
        Some("… 2 more (show them with --limit 3)")
    );
}

/// The words of a table row: "+8.0 K  20 K  +1  X/grow" → ["+8.0", "K", "20", ...].
fn words(l: &str) -> Vec<&str> {
    l.split_whitespace().collect()
}

/// Two scans of a tree where one folder stays, one goes, one is new and one
/// grows by a file.
fn changed_tree() -> Env {
    let e = Env::new();
    let r = e.root();
    let file = |rel: &str, kb: usize| {
        fs::create_dir_all(r.join(rel).parent().unwrap()).unwrap();
        fs::write(r.join(rel), vec![1u8; kb << 10]).unwrap();
    };
    file("keep/a", 8);
    file("old/b", 16);
    file("grow/c", 8);
    e.scan(0);
    fs::remove_dir_all(r.join("old")).unwrap();
    file("fresh/d", 32);
    file("grow/c2", 8);
    e.scan(0);
    e
}

#[test]
fn diff_shows_changes_with_badges() {
    let e = changed_tree();
    let root = e.p("");
    let t = e.ok(&["diff", &root]);
    let lines: Vec<&str> = t.lines().collect();
    let head = format!("{root}  +24 K since ");
    assert!(lines[0].starts_with(&head), "{t}");
    assert!(
        lines[0].ends_with(" (generation 1) · depth 1 · folders"),
        "{t}"
    );
    let columns = lines[1].trim_start();
    assert!(columns.starts_with("DELTA  ALLOCATED  ITEMS  "), "{t}");
    let fresh = words(line_ending(&t, &format!("  {root}/fresh")));
    assert_eq!(fresh[4..], ["+1", "new", &e.p("fresh")]);
    let old = words(line_ending(&t, &format!("  {root}/old")));
    assert_eq!(old[2..], ["0", "B", "−1", "gone", &e.p("old")]);
    let grow = words(line_ending(&t, &format!("  {root}/grow")));
    assert_eq!(grow[..2], ["+8.0", "K"]);
    assert_eq!(grow[4..], ["+1", &e.p("grow")]);
    assert_eq!(lines.last(), Some(&"1 unchanged not shown"), "{t}");

    let t = e.ok(&["diff", &root, "--depth", "2", "--kind", "file"]);
    let lines: Vec<&str> = t.lines().collect();
    assert!(lines[0].ends_with(" · depth 2 · files"), "{t}");
    let d = words(line_ending(&t, &format!("  {root}/fresh/d")));
    assert_eq!(d, ["+32", "K", "32", "K", "new", &e.p("fresh/d")]);
    let b = words(line_ending(&t, &format!("  {root}/old/b")));
    assert_eq!(b, ["−16", "K", "0", "B", "gone", &e.p("old/b")]);
    assert_eq!(lines.last(), Some(&"2 unchanged not shown"), "{t}");

    let t = e.ok(&["diff", &root, "--limit", "1"]);
    assert_eq!(t.lines().count(), 4, "{t}");
    let more = "… 3 more (show them with --limit 4)";
    assert_eq!(t.lines().last(), Some(more), "{t}");

    let t = e.ok(&["diff", &e.p("keep")]);
    assert_eq!(t.lines().skip(1).collect::<Vec<_>>(), ["No changes."]);
}

#[test]
fn diff_notes_a_refreshed_folder_and_other_roots() {
    let e = changed_tree();
    let root = e.p("");
    e.ok(&["refresh", &e.p("grow")]);
    let t = e.ok(&["diff", &root]);
    let partial = format!(
        "partial: {} rescanned after the previous scan, deltas there mix two baselines",
        e.p("grow")
    );
    assert_eq!(t.lines().nth(1), Some(partial.as_str()), "{t}");

    let q = e.tmp.path().join("q");
    fs::create_dir(&q).unwrap();
    e.ok(&["scan", &root, q.to_str().unwrap()]);
    let t = e.ok(&["diff", &root]);
    let changed = "roots changed: the two scans covered different folders";
    assert_eq!(t.lines().nth(1), Some(changed), "{t}");
}

#[test]
fn status_says_a_root_is_not_mounted_now_when_its_path_is_gone() {
    let e = Env::new();
    e.tree(0);
    e.scan(0);
    fs::rename(e.root(), e.tmp.path().join("moved")).unwrap();
    let t = e.ok(&["status"]);
    let (first, rows) = sentence_and_rows(&t);
    assert!(first.starts_with("? free"), "{t}");
    assert!(row(&rows, "root").ends_with(", not mounted now"), "{t}");
}

#[test]
fn a_hardlink_with_no_owner_in_the_snapshot_says_so() {
    let e = Env::new();
    e.tree(0);
    e.scan(0);
    // both links carry the alias flag, so no row is the one that counts the inode
    let db = rusqlite::Connection::open(e.snap().join("snapshot.db")).unwrap();
    db.execute(
        "UPDATE entries SET flags = flags | ?1 \
         WHERE name IN (CAST('top.bin' AS BLOB), CAST('top-link.bin' AS BLOB))",
        [i64::from(freespace_core::scan::tree::flags::ALIAS)],
    )
    .unwrap();
    drop(db);
    let t = e.ok(&["ls", &e.p("")]);
    line_ending(&t, "  top.bin  [hardlink, counted elsewhere]");
}

#[test]
fn ls_says_a_fifo_is_a_special_file() {
    let e = Env::new();
    e.tree(0);
    // a FIFO, not a socket: a socket's path may not be longer than 107 bytes
    let made = Command::new("mkfifo")
        .arg(e.root().join("fifo"))
        .status()
        .unwrap();
    assert!(made.success());
    e.scan(0);
    let t = e.ok(&["ls", &e.p("fifo")]);
    let note = format!("{} is a special file, not a folder.\n", e.p("fifo"));
    assert!(t.ends_with(&note), "{t}");
}

/// One and five unreadable folders are named in full, with no "and N more".
#[test]
fn a_few_unreadable_folders_are_named_in_full() {
    for k in [1, 5] {
        let e = Env::new();
        // as root every folder is readable, so none is named
        let n = if locked() > 0 { k } else { 0 };
        e.tree(n);
        e.scan(n);
        let t = e.ok(&["status"]);
        let (_, rows) = sentence_and_rows(&t);
        if n == 0 {
            assert!(!rows.iter().any(|(label, _)| label == "incomplete"), "{t}");
            continue;
        }
        let list: Vec<String> = (1..=n).map(|i| e.p(&format!("locked{i}"))).collect();
        let list = list.join(", ");
        let noun = if n == 1 { "directory" } else { "directories" };
        assert_eq!(
            row(&rows, "warning"),
            format!("{n} {noun} could not be read, sizes under them are unknown: {list}")
        );
        assert_eq!(
            row(&rows, "incomplete"),
            format!("unreadable, sizes unknown: {list}")
        );
        let err = e.run(&["ls", &e.p("")]).stderr;
        assert_eq!(
            String::from_utf8_lossy(&err),
            format!(
                "warning: incomplete snapshot, sizes under these folders are unknown: {list}\n"
            )
        );
    }
}

/// Two roots: the free space in the first line is for the first one, and each has its row.
#[test]
fn status_names_the_root_its_free_space_is_for_when_there_are_two() {
    let e = Env::new();
    e.tree(0);
    let q = e.tmp.path().join("q");
    fs::create_dir(&q).unwrap();
    e.ok(&["scan", &e.p(""), q.to_str().unwrap()]);
    let t = e.ok(&["status"]);
    let (first, rows) = sentence_and_rows(&t);
    assert!(first.contains(&format!(" free on {}", e.p(""))), "{t}");
    let roots: Vec<&String> = rows
        .iter()
        .filter(|(label, _)| label == "root")
        .map(|(_, value)| value)
        .collect();
    assert_eq!(roots.len(), 2, "{t}");
}

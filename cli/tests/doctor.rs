//! The failing checks of `doctor`, pinned before `cli/src/doctor.rs` is split
//! (D0 of docs/designs/code-quality-gates.md): each by its detail and fix
//! under `--json`. No snapshot, a stale one, one this version cannot read, too
//! little space for the next scan, a failed run, an unreadable state.json. The
//! checks of the host (timer, unit, binary) answer for the machine and are left
//! out.

use std::fs;

use freespace_core::snapshot::SnapshotDir;
use freespace_core::snapshot::state::{self, LastError};
use serde_json::Value;

mod common;

use common::Env;

impl Env {
    /// A small tree, scanned once.
    fn scanned() -> Self {
        let e = Env::new();
        let r = e.root();
        fs::create_dir_all(r.join("a")).unwrap();
        fs::write(r.join("a/f"), vec![1u8; 64 << 10]).unwrap();
        let o = e.run(&["scan", r.to_str().unwrap()]);
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        e
    }

    /// Every check of one run, by id; and its exit code.
    fn doctor(&self) -> (Vec<Value>, i32) {
        let o = self.run(&["doctor", "--json"]);
        let v: Value = serde_json::from_slice(&o.stdout).unwrap();
        (
            v["checks"].as_array().unwrap().clone(),
            o.status.code().unwrap(),
        )
    }

    /// (ok, detail, fix) of the check `id`.
    fn check(&self, id: &str) -> (bool, String, Option<String>) {
        let (checks, _) = self.doctor();
        let c = checks
            .iter()
            .find(|c| c["id"] == id)
            .unwrap_or_else(|| panic!("no {id} check in {checks:?}"));
        (
            c["ok"].as_bool().unwrap(),
            c["detail"].as_str().unwrap().to_owned(),
            c["fix"].as_str().map(str::to_owned),
        )
    }

    fn sql(&self, stmt: &str) {
        let db = rusqlite::Connection::open(self.snap().join("snapshot.db")).unwrap();
        db.execute_batch(stmt).unwrap();
    }

    /// state.json with a failed `op` a minute after the last success.
    fn failed(&self, op: &str, message: &str) {
        self.failed_at(60, op, message);
    }

    /// state.json with a failed `op`, `after` seconds past the last success.
    fn failed_at(&self, after: i64, op: &str, message: &str) {
        let dir = SnapshotDir::new(self.snap());
        let mut st = state::read(&dir).unwrap().unwrap();
        st.last_error = Some(LastError {
            ts: st.last_success_ts.unwrap() + after,
            op: op.into(),
            message: message.into(),
        });
        state::write(&dir, &st).unwrap();
    }
}

const JOURNAL: &str = "journalctl -u freespace.service -n 50";

#[test]
fn without_a_snapshot_it_says_how_to_make_one() {
    let e = Env::new();
    let (ok, detail, fix) = e.check("access");
    assert!(!ok);
    assert_eq!(detail, format!("no snapshot in {}", e.snap().display()));
    assert_eq!(
        fix.as_deref(),
        Some("sudo systemctl start freespace.service (or sudo freespace install-timer)")
    );
    assert_eq!(
        e.check("last_error"),
        (true, "no state.json yet".into(), None)
    );
    // no snapshot dir: nothing to measure the next scan's space on
    let (checks, code) = e.doctor();
    assert!(checks.iter().all(|c| c["id"] != "space"), "{checks:?}");
    assert_eq!(code, 1);
}

#[test]
fn a_stale_snapshot_and_too_little_space_for_the_next_scan() {
    let e = Env::scanned();
    // three days old, and as many entries as no disk has room for three times
    e.sql(
        "UPDATE meta SET scanned_at = scanned_at - 3 * 86400, entries = 1024 * 1024 * 1024 * 1024",
    );
    let (ok, detail, fix) = e.check("freshness");
    assert!(!ok);
    assert_eq!(detail, "snapshot is 3 d old (generation 1)");
    assert_eq!(
        fix.as_deref(),
        Some(format!("systemctl list-timers freespace.timer; {JOURNAL}").as_str())
    );
    let (ok, detail, fix) = e.check("space");
    assert!(!ok);
    let head = format!(
        "not enough space for a new snapshot in {}: need ",
        e.snap().display()
    );
    assert!(detail.starts_with(&head), "{detail}");
    assert!(
        detail.contains(" (3 × ") && detail.contains(" + 512 M reserve), have "),
        "{detail}"
    );
    assert_eq!(
        fix.as_deref(),
        Some(
            "free space on that filesystem, set keep_previous = false, or move snapshot_dir in /etc/freespace/config.toml"
        )
    );
    // status sends the reader here
    let o = e.cmd(&["status"]).output().unwrap();
    let t = String::from_utf8_lossy(&o.stdout);
    assert!(
        t.lines().next().unwrap().contains(" · snapshot 3 d old"),
        "{t}"
    );
    assert_eq!(t.lines().last(), Some("Run: freespace doctor"), "{t}");
}

#[test]
fn a_snapshot_this_version_cannot_read() {
    let e = Env::scanned();
    e.sql("DELETE FROM meta");
    let (ok, detail, fix) = e.check("access");
    assert!(!ok);
    assert_eq!(
        detail,
        "snapshot schema mismatch (meta table is empty), rescan needed"
    );
    assert_eq!(
        fix.as_deref(),
        Some("sudo systemctl start freespace.service to rescan with this version")
    );
    // an error with no fix of its own falls back to reinstalling
    fs::write(e.snap().join("snapshot.db"), vec![7u8; 4096]).unwrap();
    let (ok, detail, fix) = e.check("access");
    assert!(!ok);
    assert_eq!(detail, "sqlite: file is not a database");
    assert_eq!(fix.as_deref(), Some("sudo freespace install-timer"));
}

#[test]
fn a_failed_run_names_its_step_and_its_fix() {
    let e = Env::scanned();
    assert_eq!(
        e.check("last_error"),
        (true, "none since the last successful scan".into(), None)
    );
    e.failed("precheck", "not enough space for a new snapshot");
    let (ok, detail, fix) = e.check("last_error");
    assert!(!ok);
    assert!(
        detail.ends_with(" precheck: not enough space for a new snapshot"),
        "{detail}"
    );
    let room = format!(
        "free space, or set keep_previous = false / move snapshot_dir in {}, then sudo freespace install-timer",
        e.config().display()
    );
    assert_eq!(fix, Some(room.clone()));
    // reserve is another out-of-space step: the same fix as precheck
    e.failed("reserve", "disk full while reserving");
    let (ok, detail, fix) = e.check("last_error");
    assert!(!ok);
    assert!(
        detail.ends_with(" reserve: disk full while reserving"),
        "{detail}"
    );
    assert_eq!(fix, Some(room));
    e.failed("write", "disk full");
    let (ok, detail, fix) = e.check("last_error");
    assert!(!ok);
    assert!(detail.ends_with(" write: disk full"), "{detail}");
    assert_eq!(fix.as_deref(), Some(JOURNAL));

    fs::write(e.snap().join("state.json"), "{nope").unwrap();
    let (ok, detail, fix) = e.check("last_error");
    assert!(!ok);
    assert!(
        detail.starts_with("state.json unreadable: state.json is corrupt: "),
        "{detail}"
    );
    assert_eq!(fix.as_deref(), Some("sudo freespace install-timer"));
}

#[test]
fn a_failed_first_run_has_no_success_to_compare_with() {
    let e = Env::new();
    fs::create_dir_all(e.snap()).unwrap();
    let st = state::State {
        last_error: Some(LastError {
            ts: 1_700_000_000,
            op: "scan".into(),
            message: "no roots to scan".into(),
        }),
        ..Default::default()
    };
    state::write(&SnapshotDir::new(e.snap()), &st).unwrap();
    let (ok, detail, fix) = e.check("last_error");
    assert!(!ok);
    assert!(detail.ends_with(" scan: no roots to scan"), "{detail}");
    assert_eq!(fix.as_deref(), Some(JOURNAL));
}

#[test]
fn a_failed_run_with_no_recorded_success_sends_status_to_doctor() {
    let e = Env::scanned();
    let dir = SnapshotDir::new(e.snap());
    let mut st = state::read(&dir).unwrap().unwrap();
    st.last_success_ts = None;
    st.last_error = Some(LastError {
        ts: 1_700_000_000,
        op: "write".into(),
        message: "disk full".into(),
    });
    state::write(&dir, &st).unwrap();
    let (ok, _, fix) = e.check("last_error");
    assert!(!ok);
    assert_eq!(fix.as_deref(), Some(JOURNAL));
    let o = e.cmd(&["status"]).output().unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let t = String::from_utf8_lossy(&o.stdout);
    assert!(t.contains("last error"), "{t}");
    assert!(!t.contains("last success"), "{t}");
    assert_eq!(t.lines().last(), Some("Run: freespace doctor"), "{t}");
}

/// A present error remains a failure regardless of timestamp ordering; every
/// successful scan clears `last_error`.
#[test]
fn any_recorded_error_is_a_failure_and_status_points_to_doctor() {
    let e = Env::scanned();
    for offset in [-60, 0, 60] {
        e.failed_at(offset, "write", "disk \u{1b}[31mfull");
        let (ok, detail, fix) = e.check("last_error");
        assert!(!ok, "offset {offset}: {detail}");
        assert!(detail.ends_with(" write: disk \u{1b}[31mfull"), "{detail}");
        assert_eq!(fix.as_deref(), Some(JOURNAL));
        let (_, code) = e.doctor();
        assert_eq!(code, 1, "offset {offset}");
        let o = e.cmd(&["status"]).output().unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        let t = String::from_utf8_lossy(&o.stdout);
        assert!(t.contains(" write: disk \\u{1b}[31mfull"), "{t}");
        assert!(!t.contains('\u{1b}'), "{t:?}");
        assert_eq!(t.lines().last(), Some("Run: freespace doctor"), "{t}");
    }
}

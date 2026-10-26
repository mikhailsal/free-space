//! Stdout delivery errors must not turn truncated output into success.
use std::fs::{self, File};
use std::os::fd::{FromRawFd, OwnedFd};
use std::process::Stdio;

mod common;
use common::Env;

fn fixture() -> Env {
    let e = Env::new();
    fs::create_dir(e.root()).unwrap();
    fs::write(e.root().join("file"), b"data").unwrap();
    fs::create_dir(e.root().join("empty")).unwrap();
    for _ in 0..2 {
        let scan = e.run(&["scan", e.root().to_str().unwrap(), "--workers", "1"]);
        assert!(scan.status.success(), "{:?}", scan.stderr);
    }
    e
}

#[test]
fn full_stdout_fails_instead_of_reporting_success() {
    let e = fixture();
    let root = e.root();
    let root = root.to_str().unwrap();
    for args in [
        vec!["ls", root],
        vec!["ls", root, "--json"],
        vec!["ls", &format!("{root}/empty")],
        vec!["top", root],
        vec!["find", root],
        vec!["find", root, "--name", "no-matching-file"],
        vec!["diff", root],
        vec!["diff", root, "--json"],
        vec!["scan", root, "--workers", "1", "--json"],
        vec!["refresh", root, "--workers", "1", "--json"],
        vec!["status"],
        vec!["status", "--json"],
        vec![],
        vec!["schema"],
        vec!["skill", "print"],
        vec!["completions", "bash"],
        vec!["--help"],
        vec!["--version"],
        vec!["doctor"],
        vec!["doctor", "--json"],
        vec!["ls", &format!("{root}/missing"), "--json"],
        vec!["--bad-option", "--json"],
    ] {
        let output = e
            .cmd(&args)
            .env("XDG_STATE_HOME", e.tmp.path().join("state"))
            .stdout(File::options().write(true).open("/dev/full").unwrap())
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(1),
            "{args:?}: {:?}",
            output.stderr
        );
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("stdout"),
            "{args:?}: {:?}",
            output.stderr
        );
    }
}

#[test]
fn closed_stdout_pipe_is_ignored() {
    let e = fixture();
    let root = e.root();
    let root = root.to_str().unwrap();
    for (args, exit) in [
        (vec!["ls", root], 0),
        (vec!["ls", root, "--json"], 0),
        (vec!["schema"], 0),
        (vec!["--help"], 0),
        (vec!["--version"], 0),
        (vec!["completions", "bash"], 0),
        (vec!["ls", &format!("{root}/missing"), "--json"], 2),
        (vec!["--bad-option", "--json"], 2),
    ] {
        let mut fds = [-1; 2];
        // SAFETY: pipe initializes exactly two valid descriptors on success.
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
        // SAFETY: each successful pipe descriptor is owned once and closed by RAII.
        let read = unsafe { OwnedFd::from_raw_fd(fds[0]) };
        let write = unsafe { OwnedFd::from_raw_fd(fds[1]) };
        drop(read);
        let output = e
            .cmd(&args)
            .env("XDG_STATE_HOME", e.tmp.path().join("state"))
            .stdout(Stdio::from(write))
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(exit),
            "{args:?}: {:?}",
            output.stderr
        );
        assert!(
            !String::from_utf8_lossy(&output.stderr).contains("stdout"),
            "{args:?}: {:?}",
            output.stderr
        );
        if exit == 0 {
            assert!(output.stderr.is_empty(), "{args:?}: {:?}", output.stderr);
        }
    }
}

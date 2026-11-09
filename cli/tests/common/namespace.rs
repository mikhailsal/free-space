//! Root-mode scripts in private mount and PID namespaces. A script hides every host path it
//! may touch behind a tmpfs before it runs anything of ours, so a test never installs onto
//! the host, changes its groups or ACLs, or contacts its systemd.
//!
//! Included with `#[path = "common/namespace.rs"] mod namespace;` by the tests that need it,
//! so the other test crates do not compile it.

use std::process::Command;

/// Run `script` with bash as root in fresh namespaces, passing it the `freespace` binary and
/// a scratch directory. Returns its stdout once it exited 0, and panics with both streams
/// when it did not. `None` when passwordless sudo or `unshare` is missing: the test skips.
pub fn run(script: &str, label: &str) -> Option<String> {
    if !cfg!(target_os = "linux") {
        return None;
    }
    let probe = Command::new("sudo")
        .args(["-n", "unshare", "--mount", "--pid", "--fork", "/bin/true"])
        .output();
    if !probe.is_ok_and(|out| out.status.success()) {
        eprintln!("{label} skipped: passwordless sudo and mount/PID namespaces are required");
        return None;
    }
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("script.sh");
    std::fs::write(&path, script).unwrap();
    let output = Command::new("sudo")
        .args([
            "-n",
            "unshare",
            "--mount",
            "--pid",
            "--fork",
            "--mount-proc",
            "/bin/bash",
        ])
        .arg(&path)
        .arg(env!("CARGO_BIN_EXE_freespace"))
        .arg(tmp.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{label} failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

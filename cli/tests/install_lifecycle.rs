//! Root-mode installer test in private mount/PID namespaces with fake tools.
//! It must never install onto the host or contact the host's systemd.
#[test]
fn v2_install_lifecycle_in_isolated_namespaces() {
    use std::process::Command;
    if !cfg!(target_os = "linux") {
        return;
    }
    let probe = Command::new("sudo")
        .args(["-n", "unshare", "--mount", "--pid", "--fork", "/bin/true"])
        .output();
    if !probe.is_ok_and(|out| out.status.success()) {
        eprintln!(
            "root namespace test skipped: passwordless sudo and mount/PID namespaces are required"
        );
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let script = tmp.path().join("lifecycle.sh");
    std::fs::write(&script, include_str!("fixtures/install-lifecycle.sh")).unwrap();
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
        .arg(&script)
        .arg(env!("CARGO_BIN_EXE_freespace"))
        .arg(tmp.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "root lifecycle test failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("isolated v2 lifecycle passed"));
}

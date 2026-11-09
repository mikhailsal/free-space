//! Root-mode installer test in private mount/PID namespaces with fake tools.
//! It must never install onto the host or contact the host's systemd.
#[path = "common/namespace.rs"]
mod namespace;

#[test]
fn v2_install_lifecycle_in_isolated_namespaces() {
    let script = include_str!("fixtures/install-lifecycle.sh");
    let Some(stdout) = namespace::run(script, "root lifecycle test") else {
        return;
    };
    assert!(stdout.contains("isolated v2 lifecycle passed"), "{stdout}");
}

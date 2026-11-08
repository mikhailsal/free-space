//! A manual `sudo freespace scan` gives the user behind sudo read access when no timer was
//! installed. Root-mode, in private mount/PID namespaces with fake `setfacl` and `usermod`:
//! it must never touch the host's /etc, groups or ACLs.
#[path = "common/namespace.rs"]
mod namespace;

#[test]
fn a_manual_scan_gives_the_sudo_user_read_access_when_no_timer_is_installed() {
    let Some(stdout) = namespace::run(include_str!("fixtures/scan-access.sh"), "scan access test")
    else {
        return;
    };
    assert!(stdout.contains("scan access passed"), "{stdout}");
}

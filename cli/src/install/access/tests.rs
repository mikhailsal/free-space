use std::path::PathBuf;

use super::*;

fn user(name: &str, uid: u32) -> UserInfo {
    UserInfo {
        name: name.into(),
        uid,
        gid: uid,
        home: PathBuf::from("/home").join(name),
    }
}

#[test]
fn the_user_behind_sudo_gets_access_when_no_timer_is_installed() {
    let alice = user("alice", 1000);
    assert_eq!(grantee(Some(alice.clone()), false), Some(alice));
}

#[test]
fn root_scans_without_a_sudo_user_grant_nobody() {
    // The timer and cron run as root with no $SUDO_USER; `sudo -i` as root names root itself.
    assert_eq!(grantee(None, false), None);
    assert_eq!(grantee(Some(user("root", 0)), false), None);
}

#[test]
fn an_installed_timer_keeps_the_choice_the_installer_made() {
    assert_eq!(grantee(Some(user("alice", 1000)), true), None);
}

#[test]
fn each_outcome_says_what_was_granted_and_what_to_do_next() {
    let dir = SnapshotDir::new("/srv/snap");
    let acl = message("alice", &dir, "acl", false);
    assert!(
        acl.contains("alice") && acl.contains("/srv/snap") && acl.contains("ACL"),
        "{acl}"
    );
    let joined = message("alice", &dir, "group", true);
    assert!(
        joined.contains("added alice to group freespace"),
        "{joined}"
    );
    assert!(
        joined.contains("log out") && joined.contains("newgrp freespace"),
        "{joined}"
    );
    let member = message("alice", &dir, "group", false);
    assert!(
        member.contains("through group freespace") && !member.contains("log out"),
        "{member}"
    );
    let none = message("alice", &dir, "none", false);
    assert!(
        none.starts_with("warning:") && none.contains("sudo freespace status"),
        "{none}"
    );
}

#[test]
fn control_characters_in_a_name_or_path_cannot_reach_the_terminal() {
    let dir = SnapshotDir::new("/srv/sn\x1b[2Jap");
    for access in ["acl", "group", "none"] {
        let text = message("al\x1b[2Jice", &dir, access, true);
        assert!(!text.contains('\x1b'), "{access}: {text:?}");
    }
}

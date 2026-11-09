//! Read access for whoever runs `sudo freespace scan` by hand.
//!
//! `install-timer` gives the sudo user read access to the snapshots: an ACL, else the
//! `freespace` group. A machine without systemd cannot run it, and a machine installed with
//! `--no-timer` never did, so a manual scan would publish a snapshot that only root can read,
//! and the `SNAPSHOT_ACCESS` fix names the very command that refuses to run there. After a
//! scan this does for the person behind `sudo` what the installer would have done.

use freespace_core::Result;
use freespace_core::config::Config;
use freespace_core::platform::{self, UserInfo};
use freespace_core::snapshot::{SnapshotDir, fsutil};
use freespace_core::systemd::{self, TIMER};

use crate::output;

/// Who a finished root scan owes read access, if anybody.
///
/// - No invoker, or root itself: the timer or cron running as root, which reads everything.
/// - A timer unit exists: `install-timer` ran and chose the reader (`--user` may name somebody
///   else), so a manual scan leaves that choice alone.
fn grantee(invoker: Option<UserInfo>, timer_installed: bool) -> Option<UserInfo> {
    invoker.filter(|u| !timer_installed && u.uid != 0)
}

/// Grant the user behind `sudo` read access to the snapshot this scan just published.
///
/// A scan by anyone but root writes into a directory of that user's own (tests, a custom
/// `FREESPACE_SNAPSHOT_DIR`) and changes nothing. A failure only warns: the snapshot is
/// published, and root can still read it.
pub(crate) fn after_scan(config: &Config) {
    if !fsutil::is_root() {
        return;
    }
    let timer = systemd::unit_dir().join(TIMER).exists();
    let Some(user) = grantee(platform::invoking_user(), timer) else {
        return;
    };
    match grant(config, &user) {
        Ok((dir, access, relogin)) => eprintln!("{}", message(&user.name, &dir, access, relogin)),
        Err(e) => eprintln!(
            "warning: could not give {} read access to the snapshot: {}",
            output::escape(&user.name),
            output::escape(&e.message)
        ),
    }
}

/// The installer's steps for one user. Repeating them is harmless, and they also cover a
/// snapshot that an earlier release left unreadable.
fn grant(config: &Config, user: &UserInfo) -> Result<(SnapshotDir, &'static str, bool)> {
    // A missing group only rules out the group fallback; an ACL needs none.
    if let Err(e) = super::ensure_group() {
        eprintln!("note: {}", output::escape(&e.message));
    }
    let dir = super::lifecycle::prepare_backend_dirs(config)?;
    let (access, relogin) = super::lifecycle::grant_backend_access(config, &dir, user);
    Ok((dir, access, relogin))
}

/// What the scan tells its caller about the access it set up.
fn message(user: &str, dir: &SnapshotDir, access: &str, relogin: bool) -> String {
    let user = output::escape(user);
    let dir = output::escape(&dir.path.display().to_string());
    let group = fsutil::GROUP;
    match (access, relogin) {
        ("acl", _) => {
            format!(
                "freespace scan: {user} can read the snapshot in {dir} now (ACL); try: freespace status"
            )
        }
        ("group", true) => format!(
            "freespace scan: added {user} to group {group}; log out and back in (or run newgrp {group}), then: freespace status"
        ),
        ("group", false) => format!(
            "freespace scan: {user} can read the snapshot in {dir} through group {group}; try: freespace status"
        ),
        _ => format!(
            "warning: {user} cannot read the snapshot in {dir}; read it as root: sudo freespace status"
        ),
    }
}

#[cfg(test)]
mod tests;

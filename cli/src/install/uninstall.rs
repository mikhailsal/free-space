//! Remove the installation manifest in its established cleanup order.

use std::fs;
use std::path::Path;

use freespace_core::config::{self, Config};
use freespace_core::platform;
use freespace_core::snapshot::{SnapshotDir, fsutil, lock};
use freespace_core::systemd::{self, BIN_TARGET, SERVICE, TIMER};

use super::{COMPLETIONS, InstallArgs, systemctl};
use crate::{output, skill};

/// The DX manifest (OV-8), in order. Only files freespace writes are removed;
/// a snapshot dir holding anything else is left in place.
pub(super) fn uninstall(a: &InstallArgs) -> output::Result<()> {
    let cfg_path = config::config_path();
    let mut c = config::load_from(&cfg_path)
        .map(|l| l.config)
        .unwrap_or_else(|_| Config::default());
    if let Ok(loaded) = crate::backend::load() {
        c.backend = loaded.config.backend;
    }
    let mut removed: Vec<String> = Vec::new();
    let mut kept: Vec<String> = Vec::new();

    let _ = systemctl(&["disable", "--now", TIMER]);
    let _ = systemctl(&["stop", SERVICE]);
    // A manual writer is independent of systemd. Refuse cleanup while it runs.
    let v2_dir = SnapshotDir::new(c.snapshot_dir.join("v2"));
    let _writer = if c.backend == config::Backend::V2
        && v2_dir.as_path().is_dir()
        && !v2_dir.path.is_symlink()
    {
        Some(lock::acquire_with_path_arg(&v2_dir, "uninstall", "", None)?)
    } else {
        None
    };
    let _readers = if c.backend == config::Backend::V2 && !v2_dir.path.is_symlink() {
        super::lifecycle::reader_lease_for_cleanup(&v2_dir)?
    } else {
        None
    };
    let unit_dir = systemd::unit_dir();
    for u in [TIMER, SERVICE] {
        remove_file(&unit_dir.join(u), &mut removed);
    }
    let _ = systemctl(&["daemon-reload"]);
    let _ = systemctl(&["reset-failed", SERVICE]);

    remove_file(&cfg_path, &mut removed);
    if let Some(parent) = cfg_path.parent()
        && parent == Path::new("/etc/freespace")
    {
        remove_empty_dir(parent, &mut removed, &mut kept);
    }

    remove_snapshots(&c, &mut removed, &mut kept);

    for p in COMPLETIONS {
        remove_file(Path::new(p), &mut removed);
    }
    let user = match &a.user {
        Some(n) => platform::user_by_name(n),
        None => platform::invoking_user(),
    };
    if let Some(u) = &user {
        let sd = skill::default_dir_for(u);
        for f in skill::FILES {
            remove_file(&sd.join(f), &mut removed);
        }
        remove_empty_dir(&sd, &mut removed, &mut kept);
    }
    remove_file(Path::new(BIN_TARGET), &mut removed);

    print_report(&removed, &kept, a.json)
}

fn print_report(removed: &[String], kept: &[String], json: bool) -> output::Result<()> {
    if json {
        output::print_json(
            &serde_json::json!({"removed": removed, "kept": kept, "group_kept": fsutil::GROUP}),
        )?;
        return Ok(());
    }
    let mut out = String::from("freespace uninstalled\n");
    for r in removed {
        out.push_str(&format!("  removed  {}\n", output::escape(r)));
    }
    for k in kept {
        out.push_str(&format!(
            "  kept     {} (not empty: holds files freespace did not write)\n",
            output::escape(k)
        ));
    }
    out.push_str(&format!(
        "The group {} stays (harmless): sudo groupdel {} removes it.\n",
        fsutil::GROUP,
        fsutil::GROUP
    ));
    output::print(&out)?;
    Ok(())
}

fn remove_file(p: &Path, removed: &mut Vec<String>) {
    if fs::remove_file(p).is_ok() {
        removed.push(p.display().to_string());
    }
}

fn remove_empty_dir(p: &Path, removed: &mut Vec<String>, kept: &mut Vec<String>) {
    if !p.exists() {
        return;
    }
    match fs::remove_dir(p) {
        Ok(()) => removed.push(format!("{}/", p.display())),
        Err(_) => kept.push(format!("{}/", p.display())),
    }
}

fn remove_snapshots(c: &Config, removed: &mut Vec<String>, kept: &mut Vec<String>) {
    let dir = SnapshotDir::new(&c.snapshot_dir);
    if c.backend == config::Backend::V2 {
        remove_v2_snapshots(&dir, removed, kept);
        return;
    }
    for f in [
        dir.current(),
        dir.prev(),
        dir.prev_tmp(),
        dir.state(),
        dir.lock(),
        dir.progress(),
    ] {
        remove_file(&f, removed);
    }
    if let Ok(rd) = fs::read_dir(dir.as_path()) {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().into_owned();
            let ours = (n.starts_with("snapshot.") && n.ends_with(".tmp"))
                || (n.starts_with("crash-") && n.ends_with(".log"));
            if ours {
                remove_file(&e.path(), removed);
            }
        }
    }
    remove_empty_dir(dir.as_path(), removed, kept);
}

fn remove_v2_snapshots(base: &SnapshotDir, removed: &mut Vec<String>, kept: &mut Vec<String>) {
    let dir = SnapshotDir::new(base.path.join("v2"));
    // Never follow a replaced/symlinked backend directory during cleanup.
    if dir.path.is_symlink() {
        kept.push(format!("{}/", dir.path.display()));
        return;
    }
    for file in super::lifecycle::v2_files(&dir) {
        remove_file(&file, removed);
    }
    if let Ok(entries) = fs::read_dir(dir.as_path()) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if known_v2_temporary(&name) {
                remove_file(&entry.path(), removed);
            }
        }
    }
    remove_empty_dir(dir.as_path(), removed, kept);
    remove_empty_dir(base.as_path(), removed, kept);
}

fn known_v2_temporary(name: &str) -> bool {
    let crash = name
        .strip_prefix("crash-")
        .and_then(|s| s.strip_suffix(".log"))
        .is_some_and(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()));
    let stage = name
        .strip_prefix("scan-")
        .and_then(|s| s.strip_suffix(".stage.tmp"))
        .is_some_and(|s| {
            s.len() == 16
                && s.bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        });
    crash || stage
}

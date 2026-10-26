//! Remove the installation manifest in its established cleanup order.

use std::fs;
use std::path::Path;

use freespace_core::config::{self, Config};
use freespace_core::platform;
use freespace_core::snapshot::{SnapshotDir, fsutil};
use freespace_core::systemd::{self, BIN_TARGET, SERVICE, TIMER};

use super::{COMPLETIONS, InstallArgs, systemctl};
use crate::{output, skill};

/// The DX manifest (OV-8), in order. Only files freespace writes are removed;
/// a snapshot dir holding anything else is left in place.
pub(super) fn uninstall(a: &InstallArgs) -> output::Result<()> {
    let cfg_path = config::config_path();
    let c = config::load_from(&cfg_path)
        .map(|l| l.config)
        .unwrap_or_else(|_| Config::default());
    let mut removed: Vec<String> = Vec::new();
    let mut kept: Vec<String> = Vec::new();

    let _ = systemctl(&["disable", "--now", TIMER]);
    let _ = systemctl(&["stop", SERVICE]);
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

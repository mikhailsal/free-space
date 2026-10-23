//! `install-timer` (T12, T21, T37, T44): copy the binary to /usr/local/bin
//! (D22), write config.toml, create the group and snapshot dir with read access
//! for the user (ACL, else group, D8), render and enable the systemd timer, and
//! start the first scan (D5/D13). `--uninstall` removes the whole manifest.

use std::fs;
use std::io::Write;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use clap::Args;
use freespace_core::config::{self, Config};
use freespace_core::platform::{self, UserInfo};
use freespace_core::snapshot::reader::Snapshot;
use freespace_core::snapshot::schema::SCHEMA_VERSION;
use freespace_core::snapshot::{SnapshotDir, fsutil, lock};
use freespace_core::systemd::{self, BIN_TARGET, SERVICE, TIMER};
use freespace_core::{Error, ErrorCode, Result};
use serde::Serialize;

use crate::output;
use crate::skill;

/// Completion files install.sh writes (D19); uninstall removes them.
pub const COMPLETIONS: [&str; 3] = [
    "/usr/local/share/bash-completion/completions/freespace",
    "/usr/local/share/zsh/site-functions/_freespace",
    "/usr/local/share/fish/vendor_completions.d/freespace.fish",
];

#[derive(Args)]
pub struct InstallArgs {
    /// Roots to scan (saved to config.toml; default: every real filesystem)
    #[arg(long, num_args = 1.., value_hint = clap::ValueHint::DirPath)]
    pub roots: Vec<PathBuf>,
    /// How often the timer runs
    #[arg(long, value_parser = ["hourly", "daily", "weekly"])]
    pub interval: Option<String>,
    /// Local time of the run, HH:MM (minute only for hourly)
    #[arg(long, value_name = "HH:MM")]
    pub at: Option<String>,
    /// Keep the previous snapshot for `diff` (needs 3x snapshot size free)
    #[arg(long, value_name = "BOOL", num_args = 0..=1, default_missing_value = "true")]
    pub keep_previous: Option<bool>,
    /// Where snapshots live (default /var/lib/freespace)
    #[arg(long, value_hint = clap::ValueHint::DirPath)]
    pub snapshot_dir: Option<PathBuf>,
    /// User who gets read access to the snapshots (default: $SUDO_USER)
    #[arg(long, value_name = "NAME")]
    pub user: Option<String>,
    /// Do not start a scan now even when there is no snapshot yet
    #[arg(long)]
    pub no_start: bool,
    /// Remove the timer, config, snapshots, completions, the skill and the binary
    #[arg(long)]
    pub uninstall: bool,
    #[arg(long)]
    pub json: bool,
}

#[derive(Serialize)]
struct Report {
    binary: String,
    /// Where the binary was copied from; null when it already ran from there.
    copied_from: Option<String>,
    config: String,
    roots: Vec<String>,
    interval: String,
    at: String,
    keep_previous: bool,
    snapshot_dir: String,
    user: Option<String>,
    /// "acl", "group" (re-login needed) or "none".
    access: String,
    relogin_needed: bool,
    next_run: Option<String>,
    scan_started: bool,
    scan_reason: Option<String>,
}

pub fn run(a: InstallArgs) -> Result<()> {
    if !fsutil::is_root() {
        return Err(Error::new(ErrorCode::NotRoot, "install-timer needs root")
            .with_fix("sudo freespace install-timer"));
    }
    systemd::check_supported()?;
    if a.uninstall {
        return uninstall(&a);
    }
    let cfg_path = config::config_path();
    let mut c = config::load_from(&cfg_path)?.config;
    if !a.roots.is_empty() {
        c.roots = a.roots.clone();
    }
    if let Some(v) = &a.interval {
        c.interval = v.clone();
    }
    if let Some(v) = &a.at {
        c.at = v.clone();
    }
    if let Some(v) = a.keep_previous {
        c.keep_previous = v;
    }
    if let Some(v) = &a.snapshot_dir {
        c.snapshot_dir = v.clone();
    }
    if c.roots.is_empty() {
        c.roots = platform::default_roots(&platform::mountinfo());
    }
    let text = config::render(&c);
    // the file we write must load back: catches a bad --at or a relative path
    config::parse(&text, &cfg_path)?;

    let copied_from = self_install()?;
    if let Some(from) = &copied_from {
        eprintln!("installed {BIN_TARGET} (copied from {from})");
    }
    if let Some(parent) = cfg_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| Error::io(format!("create {}", parent.display()), e))?;
    }
    write_root_file(&cfg_path, text.as_bytes(), 0o644)?;
    ensure_group()?;
    let dir = SnapshotDir::new(&c.snapshot_dir);
    prepare_snapshot_dir(&dir)?;

    let user = match &a.user {
        Some(n) => Some(
            platform::user_by_name(n).ok_or_else(|| Error::usage(format!("no such user {n:?}")))?,
        ),
        None => platform::invoking_user(),
    };
    let (access, relogin) = match &user {
        Some(u) => grant_access(&c.snapshot_dir, u),
        None => {
            eprintln!(
                "note: no $SUDO_USER, so nobody got read access; run: sudo freespace install-timer --user <name>"
            );
            ("none", false)
        }
    };

    let unit_dir = systemd::unit_dir();
    write_root_file(
        &unit_dir.join(SERVICE),
        systemd::render_service(&c, &cfg_path).as_bytes(),
        0o644,
    )?;
    write_root_file(
        &unit_dir.join(TIMER),
        systemd::render_timer(&c).as_bytes(),
        0o644,
    )?;
    verify_units(&unit_dir);
    systemctl(&["daemon-reload"])?;
    systemctl(&["enable", "--now", TIMER])?;
    let next_run = timer_next_run();

    let (scan_started, scan_reason) = if a.no_start {
        (false, None)
    } else {
        start_scan_if_needed(&dir)?
    };

    let report = Report {
        binary: BIN_TARGET.into(),
        copied_from,
        config: cfg_path.display().to_string(),
        roots: c.roots.iter().map(|r| r.display().to_string()).collect(),
        interval: c.interval.clone(),
        at: c.at.clone(),
        keep_previous: c.keep_previous,
        snapshot_dir: c.snapshot_dir.display().to_string(),
        user: user.as_ref().map(|u| u.name.clone()),
        access: access.into(),
        relogin_needed: relogin,
        next_run: next_run.clone(),
        scan_started,
        scan_reason: scan_reason.clone(),
    };
    if a.json {
        output::print_json(&report);
        return Ok(());
    }
    let mut out = String::new();
    out.push_str(&format!(
        "freespace {} timer installed\n  config     {} (roots {}, {} at {}, keep_previous {})\n",
        freespace_core::VERSION,
        report.config,
        report.roots.join(" "),
        report.interval,
        report.at,
        report.keep_previous
    ));
    let who = match (&report.user, access) {
        (Some(u), "acl") => format!("read access for {u} (ACL)"),
        (Some(u), "group") => format!("read access for {u} via group freespace"),
        _ => "root only".into(),
    };
    out.push_str(&format!("  snapshots  {} ({who})\n", report.snapshot_dir));
    out.push_str(&format!(
        "  timer      {TIMER}, next run {}\n",
        next_run.as_deref().unwrap_or("unknown")
    ));
    if relogin {
        out.push_str(&format!(
            "Log out and back in once so {} picks up the freespace group.\n",
            report.user.as_deref().unwrap_or("the user")
        ));
    }
    match (scan_started, scan_reason.as_deref()) {
        (true, Some(why)) => out.push_str(&format!(
            "First scan started ({why}) · Open: freespace gui\n"
        )),
        (false, Some(why)) => out.push_str(&format!("{why}\n")),
        _ => {}
    }
    output::print(&out);
    Ok(())
}

/// D22: the unit runs /usr/local/bin/freespace; copy ourselves there unless we
/// already are that file. Refuses when the target directory or any parent is
/// writable by a non-root user.
fn self_install() -> Result<Option<String>> {
    let target = Path::new(BIN_TARGET);
    let parent = target.parent().unwrap_or(Path::new("/"));
    fs::create_dir_all(parent).map_err(|e| Error::io(format!("create {}", parent.display()), e))?;
    check_root_only(parent)?;
    let exe = std::env::current_exe().map_err(|e| Error::io("locate the running binary", e))?;
    if exe == target {
        let m = fs::metadata(target).map_err(|e| Error::io(format!("stat {BIN_TARGET}"), e))?;
        if m.uid() != 0 || m.mode() & 0o022 != 0 {
            return Err(Error::generic(format!(
                "{BIN_TARGET} is not root-owned or is writable by others; the timer would run it as root"
            ))
            .with_fix(format!("sudo chown root:root {BIN_TARGET} && sudo chmod 0755 {BIN_TARGET}")));
        }
        return Ok(None);
    }
    let tmp = parent.join(format!(".freespace.{}.tmp", std::process::id()));
    let res = (|| -> std::io::Result<()> {
        fs::copy(&exe, &tmp)?;
        let f = fs::File::open(&tmp)?;
        std::os::unix::fs::fchown(&f, Some(0), Some(0))?;
        f.set_permissions(fs::Permissions::from_mode(0o755))?;
        f.sync_all()?;
        fs::rename(&tmp, target)
    })();
    if let Err(e) = res {
        let _ = fs::remove_file(&tmp);
        return Err(Error::io(format!("install {BIN_TARGET}"), e));
    }
    Ok(Some(exe.display().to_string()))
}

/// Every directory from `dir` up to `/` must be root-owned and not writable by
/// group or others; otherwise a user could swap the binary the timer runs.
pub fn check_root_only(dir: &Path) -> Result<()> {
    let real =
        fs::canonicalize(dir).map_err(|e| Error::io(format!("resolve {}", dir.display()), e))?;
    for d in real.ancestors() {
        let m = fs::metadata(d).map_err(|e| Error::io(format!("stat {}", d.display()), e))?;
        if m.uid() != 0 || m.mode() & 0o022 != 0 {
            return Err(Error::generic(format!(
                "refusing to install into {}: {} is writable by a non-root user",
                dir.display(),
                d.display()
            ))
            .with_fix(format!(
                "make {} root-owned and not group/other-writable",
                d.display()
            )));
        }
    }
    Ok(())
}

fn write_root_file(path: &Path, data: &[u8], mode: u32) -> Result<()> {
    let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
    let res = (|| -> std::io::Result<()> {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(data)?;
        f.set_permissions(fs::Permissions::from_mode(mode))?;
        std::os::unix::fs::fchown(&f, Some(0), Some(0))?;
        f.sync_all()?;
        fs::rename(&tmp, path)
    })();
    res.map_err(|e| {
        let _ = fs::remove_file(&tmp);
        Error::io(format!("write {}", path.display()), e)
    })
}

fn ensure_group() -> Result<()> {
    if fsutil::group_gid(fsutil::GROUP).is_some() {
        return Ok(());
    }
    run_tool("groupadd", &["--system", fsutil::GROUP]).map(|_| ())
}

/// Snapshot dir 0750 root:freespace, existing snapshot files re-secured, so a
/// dir created by an earlier manual `sudo freespace scan` gets the right modes.
fn prepare_snapshot_dir(dir: &SnapshotDir) -> Result<()> {
    let p = dir.as_path();
    fsutil::ensure_dir(p).map_err(|e| Error::io(format!("create {}", p.display()), e))?;
    let gid = fsutil::group_gid(fsutil::GROUP);
    std::os::unix::fs::chown(p, Some(0), gid)
        .map_err(|e| Error::io(format!("chown {}", p.display()), e))?;
    fs::set_permissions(p, fs::Permissions::from_mode(fsutil::DIR_MODE))
        .map_err(|e| Error::io(format!("chmod {}", p.display()), e))?;
    for f in [
        dir.current(),
        dir.prev(),
        dir.state(),
        dir.lock(),
        dir.progress(),
    ] {
        if f.exists() {
            fsutil::secure_path(&f).map_err(|e| Error::io(format!("secure {}", f.display()), e))?;
        }
    }
    Ok(())
}

/// D8: ACL `u:NAME:rX` on the dir and its files plus the default ACL for new
/// files; without setfacl (or ACL support) fall back to the group.
fn grant_access(dir: &Path, u: &UserInfo) -> (&'static str, bool) {
    let d = dir.display().to_string();
    let spec = format!("u:{}:rX", u.name);
    let acl = run_tool("setfacl", &["-R", "-m", &spec, &d])
        .and_then(|_| run_tool("setfacl", &["-d", "-m", &spec, &d]));
    match acl {
        Ok(_) => ("acl", false),
        Err(e) => {
            eprintln!(
                "note: ACL not applied ({}); using group {} instead",
                e.message,
                fsutil::GROUP
            );
            let in_group = user_in_group(u);
            if !in_group && let Err(e) = run_tool("usermod", &["-aG", fsutil::GROUP, &u.name]) {
                eprintln!(
                    "warning: could not add {} to group {}: {}",
                    u.name,
                    fsutil::GROUP,
                    e.message
                );
                return ("none", false);
            }
            ("group", !in_group)
        }
    }
}

fn user_in_group(u: &UserInfo) -> bool {
    Command::new("id")
        .args(["-nG", &u.name])
        .output()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .split_whitespace()
                .any(|g| g == fsutil::GROUP)
        })
        .unwrap_or(false)
}

fn verify_units(unit_dir: &Path) {
    let svc = unit_dir.join(SERVICE);
    match Command::new("systemd-analyze")
        .arg("verify")
        .arg(&svc)
        .output()
    {
        Ok(o) if !o.status.success() => {
            let msg = String::from_utf8_lossy(&o.stderr);
            eprintln!(
                "warning: systemd-analyze verify {}: {}",
                svc.display(),
                msg.trim()
            );
        }
        _ => {}
    }
}

fn systemctl(args: &[&str]) -> Result<String> {
    run_tool("systemctl", args)
}

fn run_tool(tool: &str, args: &[&str]) -> Result<String> {
    let o = Command::new(tool)
        .args(args)
        .output()
        .map_err(|e| Error::generic(format!("{tool}: {e}")))?;
    if !o.status.success() {
        let err = String::from_utf8_lossy(&o.stderr);
        return Err(Error::generic(format!(
            "{tool} {}: {}",
            args.join(" "),
            err.trim()
        )));
    }
    Ok(String::from_utf8_lossy(&o.stdout).trim().to_owned())
}

pub fn timer_next_run() -> Option<String> {
    systemctl(&["show", TIMER, "-p", "NextElapseUSecRealtime", "--value"])
        .ok()
        .filter(|s| !s.is_empty() && s != "n/a")
}

/// D5/D13: no snapshot, or one this binary cannot read → start the unit now and
/// wait up to 5 s for it to take the lock (or fail).
fn start_scan_if_needed(dir: &SnapshotDir) -> Result<(bool, Option<String>)> {
    let (reason, schema_change) = match Snapshot::open_current(dir) {
        Ok(_) => return Ok((false, None)),
        Err(e) if e.code == ErrorCode::NoSnapshot => ("no snapshot yet".to_owned(), false),
        Err(e) if e.code == ErrorCode::Schema => (
            format!("snapshot schema changed to {SCHEMA_VERSION}, rescanning"),
            true,
        ),
        Err(e) => return Err(e),
    };
    let unit_active = Command::new("systemctl")
        .args(["is-active", "--quiet", SERVICE])
        .status()
        .is_ok_and(|s| s.success());
    let running = lock::probe(dir).map(|s| {
        format!(
            "A {} of {} is already running (pid {}).",
            s.op, s.path, s.pid
        )
    });
    if !schema_change && let Some(what) = running {
        return Ok((false, Some(what)));
    }
    if !schema_change && unit_active {
        return Ok((false, Some(format!("{SERVICE} is already running."))));
    }
    let _ = systemctl(&["reset-failed", SERVICE]);
    // OV-4: an old-binary scan still running would publish the old schema again
    let verb = if unit_active { "restart" } else { "start" };
    systemctl(&[verb, "--no-block", SERVICE])?;
    let t = Instant::now();
    while t.elapsed() < Duration::from_secs(5) {
        if lock::probe(dir).is_some() {
            return Ok((true, Some(reason)));
        }
        if Command::new("systemctl")
            .args(["is-failed", "--quiet", SERVICE])
            .status()
            .is_ok_and(|s| s.success())
        {
            return Err(Error::generic("the first scan failed to start")
                .with_fix(format!("journalctl -u {SERVICE} -n 50")));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    // a slow start is not a failure: the unit is queued
    Ok((true, Some(reason)))
}

/// The DX manifest (OV-8), in order. Only files freespace writes are removed;
/// a snapshot dir holding anything else is left in place.
fn uninstall(a: &InstallArgs) -> Result<()> {
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

    let dir = SnapshotDir::new(&c.snapshot_dir);
    for f in [
        dir.current(),
        dir.prev(),
        dir.prev_tmp(),
        dir.state(),
        dir.lock(),
        dir.progress(),
    ] {
        remove_file(&f, &mut removed);
    }
    if let Ok(rd) = fs::read_dir(dir.as_path()) {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().into_owned();
            let ours = (n.starts_with("snapshot.") && n.ends_with(".tmp"))
                || (n.starts_with("crash-") && n.ends_with(".log"));
            if ours {
                remove_file(&e.path(), &mut removed);
            }
        }
    }
    remove_empty_dir(dir.as_path(), &mut removed, &mut kept);

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

    if a.json {
        output::print_json(
            &serde_json::json!({"removed": removed, "kept": kept, "group_kept": fsutil::GROUP}),
        );
        return Ok(());
    }
    let mut out = String::from("freespace uninstalled\n");
    for r in &removed {
        out.push_str(&format!("  removed  {r}\n"));
    }
    for k in &kept {
        out.push_str(&format!(
            "  kept     {k} (not empty: holds files freespace did not write)\n"
        ));
    }
    out.push_str(&format!(
        "The group {} stays (harmless): sudo groupdel {} removes it.\n",
        fsutil::GROUP,
        fsutil::GROUP
    ));
    output::print(&out);
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

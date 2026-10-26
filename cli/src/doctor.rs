//! `doctor [--json]` (D12, T41): one line per check, each failure with the
//! command that fixes it; exit 0 when every check passes, 1 otherwise.

use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use clap::Args;
use freespace_core::config;
use freespace_core::format;
use freespace_core::platform::{self, now_secs};
use freespace_core::queries::types::{DoctorCheck, DoctorResponse};
use freespace_core::snapshot::publish;
use freespace_core::snapshot::reader::Snapshot;
use freespace_core::snapshot::{SnapshotDir, lock, state};
use freespace_core::systemd::{self, BIN_TARGET, SERVICE, TIMER};
use freespace_core::{ErrorCode, VERSION};

use crate::install::check_root_only;
use crate::output;
use crate::skill;

/// Snapshot older than this means the timer is not doing its job (D12).
pub const STALE_SECONDS: i64 = 26 * 3600;

#[derive(Args)]
pub struct DoctorArgs {
    #[arg(long)]
    pub json: bool,
}

pub type Check = DoctorCheck;

fn pass(id: &str, detail: impl Into<String>) -> Check {
    Check {
        id: id.into(),
        ok: true,
        detail: detail.into(),
        fix: None,
    }
}

fn fail(id: &str, detail: impl Into<String>, fix: impl Into<String>) -> Check {
    Check {
        id: id.into(),
        ok: false,
        detail: detail.into(),
        fix: Some(fix.into()),
    }
}

/// Returns whether every check passed.
pub fn run(a: &DoctorArgs) -> output::Result<bool> {
    let mut checks = Vec::new();
    let loaded = match config::load() {
        Ok(l) => {
            let what = if l.exists {
                ""
            } else {
                " (missing: defaults in use)"
            };
            checks.push(pass("config", format!("{}{what}", l.path.display())));
            Some(l)
        }
        Err(e) => {
            checks.push(fail(
                "config",
                e.message.clone(),
                e.fix
                    .clone()
                    .unwrap_or_else(|| "sudo freespace install-timer".into()),
            ));
            None
        }
    };
    let cfg = loaded
        .as_ref()
        .map(|l| l.config.clone())
        .unwrap_or_default();
    let dir = SnapshotDir::new(&cfg.snapshot_dir);

    checks.push(timer());
    checks.push(binary());
    checks.push(path_shadow());
    checks.push(unit_paths(&cfg.snapshot_dir));
    let roots = if cfg.roots.is_empty() {
        platform::default_roots(&platform::mountinfo())
    } else {
        cfg.roots.clone()
    };
    checks.extend(snapshot_checks(&dir, cfg.keep_previous, &roots));
    checks.push(skill_version());

    let ok = checks.iter().all(|c| c.ok);
    if a.json {
        output::print_json(&DoctorResponse {
            ok,
            version: VERSION.into(),
            checks,
        })?;
        return Ok(ok);
    }
    let w = checks.iter().map(|c| c.id.len()).max().unwrap_or(0);
    let mut out = String::new();
    for c in &checks {
        let mark = if c.ok { "ok  " } else { "FAIL" };
        out.push_str(&format!(
            "{mark}  {:<w$}  {}\n",
            c.id,
            output::escape(&c.detail)
        ));
        if let Some(f) = &c.fix {
            out.push_str(&format!("      {:<w$}  fix: {}\n", "", output::escape(f)));
        }
    }
    let failed = checks.iter().filter(|c| !c.ok).count();
    out.push_str(&if failed == 0 {
        "All checks passed.\n".to_owned()
    } else {
        format!("{failed} check(s) failed.\n")
    });
    output::print(&out)?;
    Ok(ok)
}

fn systemctl_value(args: &[&str]) -> Option<String> {
    let o = Command::new("systemctl").args(args).output().ok()?;
    Some(String::from_utf8_lossy(&o.stdout).trim().to_owned())
}

fn timer() -> Check {
    if !systemd::available() {
        return fail(
            "timer",
            "systemd is not running here",
            "run `sudo freespace scan` from cron or by hand",
        );
    }
    let enabled = systemctl_value(&["is-enabled", TIMER]).unwrap_or_default();
    let active = systemctl_value(&["is-active", TIMER]).unwrap_or_default();
    if enabled.is_empty() || enabled == "not-found" {
        return fail(
            "timer",
            format!("{TIMER} is not installed"),
            "sudo freespace install-timer",
        );
    }
    if enabled != "enabled" || active != "active" {
        return fail(
            "timer",
            format!("{TIMER} is {enabled}/{active}"),
            "sudo freespace install-timer",
        );
    }
    let next = crate::install::timer_next_run().unwrap_or_else(|| "unknown".into());
    let failed = Command::new("systemctl")
        .args(["is-failed", "--quiet", SERVICE])
        .status()
        .is_ok_and(|s| s.success());
    if failed {
        return fail(
            "timer",
            format!("{TIMER} active, but the last {SERVICE} run failed; next run {next}"),
            format!("journalctl -u {SERVICE} -n 50"),
        );
    }
    pass("timer", format!("active, next run {next}"))
}

/// D22: the unit's ExecStart target is root-owned and nobody else can swap it.
fn binary() -> Check {
    let t = Path::new(BIN_TARGET);
    match fs::metadata(t) {
        Err(_) => fail(
            "binary",
            format!("{BIN_TARGET} is missing"),
            "sudo freespace install-timer",
        ),
        Ok(m) if m.uid() != 0 || m.mode() & 0o022 != 0 => fail(
            "binary",
            format!(
                "{BIN_TARGET} is not root-owned or is writable by others (the timer runs it as root)"
            ),
            format!("sudo chown root:root {BIN_TARGET} && sudo chmod 0755 {BIN_TARGET}"),
        ),
        Ok(_) => match check_root_only(t.parent().unwrap_or(Path::new("/"))) {
            Err(e) => fail(
                "binary",
                e.message.clone(),
                e.fix.clone().unwrap_or_default(),
            ),
            Ok(()) => pass(
                "binary",
                format!("{BIN_TARGET} root-owned, {}", installed_version()),
            ),
        },
    }
}

fn installed_version() -> String {
    Command::new(BIN_TARGET)
        .arg("--version")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "version unknown".into())
}

/// The `freespace` a shell finds first is the installed one.
fn path_shadow() -> Check {
    let path = std::env::var_os("PATH").unwrap_or_default();
    let first: Option<PathBuf> = std::env::split_paths(&path)
        .map(|d| d.join("freespace"))
        .find(|p| p.is_file());
    match first {
        None => fail("path", "no freespace in PATH", "add /usr/local/bin to PATH"),
        Some(p) => {
            let real = fs::canonicalize(&p).unwrap_or(p.clone());
            if real == Path::new(BIN_TARGET) {
                pass("path", format!("freespace resolves to {BIN_TARGET}"))
            } else {
                fail(
                    "path",
                    format!(
                        "freespace resolves to {} and shadows {BIN_TARGET}",
                        p.display()
                    ),
                    format!("remove {} or put /usr/local/bin first in PATH", p.display()),
                )
            }
        }
    }
}

/// OV-6: the unit may write where config.toml says snapshots live.
fn unit_paths(snapshot_dir: &Path) -> Check {
    let unit = systemd::unit_dir().join(SERVICE);
    let Ok(text) = fs::read_to_string(&unit) else {
        return fail(
            "unit",
            format!("{} is missing", unit.display()),
            "sudo freespace install-timer",
        );
    };
    let rw = systemd::read_write_paths(&text);
    let want = snapshot_dir.display().to_string();
    if rw.contains(&want) {
        pass("unit", format!("{SERVICE} may write {want}"))
    } else {
        fail(
            "unit",
            format!(
                "{SERVICE} writes {} but config.toml says {want}",
                rw.join(" ")
            ),
            "sudo freespace install-timer",
        )
    }
}

fn snapshot_checks(dir: &SnapshotDir, keep_previous: bool, roots: &[PathBuf]) -> Vec<Check> {
    let mut v = Vec::new();
    let snap = Snapshot::open_current(dir);
    match &snap {
        Ok(s) => {
            v.push(pass(
                "access",
                format!("{} is readable as {}", dir.current().display(), whoami()),
            ));
            let age = now_secs() - s.meta.scanned_at;
            if age > STALE_SECONDS {
                v.push(fail(
                    "freshness",
                    format!(
                        "snapshot is {} old (generation {})",
                        format::age(age),
                        s.meta.generation
                    ),
                    format!("systemctl list-timers {TIMER}; journalctl -u {SERVICE} -n 50"),
                ));
            } else {
                v.push(pass(
                    "freshness",
                    format!(
                        "generation {} · {} old",
                        s.meta.generation,
                        format::age(age)
                    ),
                ));
            }
        }
        Err(e) => match e.code {
            ErrorCode::NoSnapshot if lock::probe(dir).is_some() => {
                v.push(pass("access", "no snapshot yet: the first scan is running"));
            }
            ErrorCode::NoSnapshot => v.push(fail(
                "access",
                format!("no snapshot in {}", dir.path.display()),
                format!("sudo systemctl start {SERVICE} (or sudo freespace install-timer)"),
            )),
            ErrorCode::Schema => v.push(fail(
                "access",
                e.message.clone(),
                format!("sudo systemctl start {SERVICE} to rescan with this version"),
            )),
            _ => v.push(fail(
                "access",
                e.message.clone(),
                e.fix
                    .clone()
                    .unwrap_or_else(|| "sudo freespace install-timer".into()),
            )),
        },
    }

    v.push(recorded_error_check(dir));

    // the next scan's precheck, computed the same way scan does it; before the
    // first snapshot, from the inodes in use on the roots (the fallback, 2 % of
    // the used space, asked for 8.1 G per snapshot where the real one is 1 G)
    let entries = match &snap {
        Ok(s) => Some(s.meta.entries),
        Err(_) => freespace_core::job::expected_entries(roots),
    };
    if let Some(check) = next_scan_space_check(dir, entries, keep_previous) {
        v.push(check);
    }
    v
}

fn recorded_error_check(dir: &SnapshotDir) -> Check {
    match state::read(dir) {
        Ok(Some(st)) => match &st.last_error {
            Some(le) => {
                let fix = match le.op.as_str() {
                    "precheck" | "reserve" => format!(
                        "free space, or set keep_previous = false / move snapshot_dir in {}, then sudo freespace install-timer",
                        config::config_path().display()
                    ),
                    _ => format!("journalctl -u {SERVICE} -n 50"),
                };
                fail(
                    "last_error",
                    format!("{} {}: {}", format::local_date(le.ts), le.op, le.message),
                    fix,
                )
            }
            None => pass("last_error", "none since the last successful scan"),
        },
        Ok(None) => pass("last_error", "no state.json yet"),
        Err(e) => fail(
            "last_error",
            format!("state.json unreadable: {e}"),
            "sudo freespace install-timer",
        ),
    }
}

fn next_scan_space_check(
    dir: &SnapshotDir,
    entries: Option<u64>,
    keep_previous: bool,
) -> Option<Check> {
    if !dir.as_path().is_dir() {
        return None;
    }
    let factor = if keep_previous { 3 } else { 2 };
    Some(match publish::precheck(dir, entries, factor) {
        Ok(i) => pass(
            "space",
            format!(
                "next scan needs {} free, {} available",
                format::size(i.need),
                format::size(i.avail)
            ),
        ),
        Err(e) => fail(
            "space",
            e.message.clone(),
            e.fix
                .clone()
                .unwrap_or_else(|| "free space or move snapshot_dir".into()),
        ),
    })
}

fn whoami() -> String {
    platform::current_user()
        .map(|u| u.name)
        .unwrap_or_else(|| "this user".into())
}

/// The installed skill matches this binary (D9). Skipped without ~/.claude.
fn skill_version() -> Check {
    let Some(u) = platform::invoking_user() else {
        return pass("skill", "no user to check (root shell)");
    };
    if !u.home.join(".claude").is_dir() {
        return pass("skill", format!("{} has no ~/.claude, skipped", u.name));
    }
    let dir = skill::default_dir_for(&u);
    match skill::installed_version(&dir) {
        Some(v) if v == VERSION => pass("skill", format!("{} at {VERSION}", dir.display())),
        Some(v) => fail(
            "skill",
            format!("{} is {v}, binary is {VERSION}", dir.display()),
            "freespace skill install",
        ),
        None => fail(
            "skill",
            format!("not installed in {}", dir.display()),
            "freespace skill install",
        ),
    }
}

//! Scheduling (T21, T37). v1 has one backend, the systemd timer; other OSes get
//! NOT_SUPPORTED from `scheduler()` instead of stub files (HOLD-2).

use std::path::{Path, PathBuf};

use crate::config::{self, Config};
use crate::error::{Error, ErrorCode, Result};

/// The unit always runs this path (D22): `install-timer` copies itself here.
pub const BIN_TARGET: &str = "/usr/local/bin/freespace";
pub const UNIT_DIR: &str = "/etc/systemd/system";
pub const SERVICE: &str = "freespace.service";
pub const TIMER: &str = "freespace.timer";

pub fn unit_dir() -> PathBuf {
    std::env::var_os("FREESPACE_UNIT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(UNIT_DIR))
}

/// systemd is the init system (`sd_booted()` semantics).
pub fn available() -> bool {
    Path::new("/run/systemd/system").is_dir()
}

pub fn check_supported() -> Result<()> {
    if !cfg!(target_os = "linux") {
        return Err(Error::new(
            ErrorCode::NotSupported,
            format!("scheduler not supported on {}", std::env::consts::OS),
        ));
    }
    if !available() {
        return Err(
            Error::new(ErrorCode::NotSupported, "systemd is not running here")
                .with_fix("run `sudo freespace scan` from cron or by hand instead"),
        );
    }
    Ok(())
}

/// `freespace.service`: hardened per D7/OV-10. No PrivateTmp (it would hide the
/// host /tmp from the scanner), no ProtectHome (the scan must read /home).
pub fn render_service(c: &Config, config_path: &Path) -> String {
    let dir = c.snapshot_dir.display().to_string();
    let mut rw = vec![quote_path(&dir)];
    if dir != config::DEFAULT_SNAPSHOT_DIR {
        rw.push(format!("-{}", config::DEFAULT_SNAPSHOT_DIR));
    }
    let env = if config_path != Path::new(config::DEFAULT_CONFIG) {
        format!(
            "Environment=FREESPACE_CONFIG={}\n",
            quote_path(&config_path.display().to_string())
        )
    } else {
        String::new()
    };
    format!(
        "# Written by `freespace install-timer`; rerun it after changing {cfg}.
[Unit]
Description=freespace disk usage snapshot
Documentation=https://github.com/mikhailsal/free-space
After=local-fs.target

[Service]
Type=oneshot
ExecStart={BIN_TARGET} scan
{env}Nice=19
IOSchedulingClass=idle
ProtectSystem=strict
ReadWritePaths={rw}
NoNewPrivileges=true
",
        cfg = config_path.display(),
        rw = rw.join(" ")
    )
}

/// `OnCalendar=` for `interval` + `at` (validated by the config loader).
pub fn on_calendar(interval: &str, at: &str) -> String {
    let (h, m) = at.split_once(':').unwrap_or(("02", "00"));
    match interval {
        "hourly" => format!("*-*-* *:{m}:00"),
        "weekly" => format!("Mon *-*-* {h}:{m}:00"),
        _ => format!("*-*-* {h}:{m}:00"),
    }
}

pub fn render_timer(c: &Config) -> String {
    format!(
        "# Written by `freespace install-timer`.
[Unit]
Description=freespace scan ({interval} at {at})

[Timer]
OnCalendar={cal}
Persistent=true
AccuracySec=1min

[Install]
WantedBy=timers.target
",
        interval = c.interval,
        at = c.at,
        cal = on_calendar(&c.interval, &c.at)
    )
}

/// systemd splits unit values on spaces; quote paths that contain them.
fn quote_path(p: &str) -> String {
    if p.contains(' ') {
        format!("\"{p}\"")
    } else {
        p.to_owned()
    }
}

/// `ReadWritePaths=` values of an installed unit, for doctor (OV-6). Values are
/// space-separated, double quotes group, a leading `-` means "ignore if missing".
pub fn read_write_paths(unit_text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for v in unit_text
        .lines()
        .filter_map(|l| l.strip_prefix("ReadWritePaths="))
    {
        parse_read_write_paths(v, &mut out);
    }
    out
}

fn parse_read_write_paths(value: &str, out: &mut Vec<String>) {
    let mut cur = String::new();
    let mut quoted = false;
    for ch in value.chars().chain(std::iter::once(' ')) {
        match ch {
            '"' => quoted = !quoted,
            ' ' | '\t' if !quoted => {
                if !cur.is_empty() {
                    out.push(cur.trim_start_matches('-').to_owned());
                    cur.clear();
                }
            }
            c => cur.push(c),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_has_the_hardening_keys() {
        let c = Config::default();
        let u = render_service(&c, Path::new(config::DEFAULT_CONFIG));
        for k in [
            "ExecStart=/usr/local/bin/freespace scan\n",
            "Nice=19\n",
            "IOSchedulingClass=idle\n",
            "ProtectSystem=strict\n",
            "ReadWritePaths=/var/lib/freespace\n",
            "NoNewPrivileges=true\n",
            "Type=oneshot\n",
        ] {
            assert!(u.contains(k), "{k} missing:\n{u}");
        }
        assert!(!u.contains("PrivateTmp"));
        assert!(!u.contains("ProtectHome"));
        assert!(!u.contains("Environment="));
    }

    #[test]
    fn moved_snapshot_dir_and_config_reach_the_unit() {
        let c = Config {
            snapshot_dir: "/data/free space".into(),
            ..Config::default()
        };
        let u = render_service(&c, Path::new("/opt/fs.toml"));
        assert!(
            u.contains("ReadWritePaths=\"/data/free space\" -/var/lib/freespace\n"),
            "{u}"
        );
        assert!(u.contains("Environment=FREESPACE_CONFIG=/opt/fs.toml\n"));
        assert_eq!(
            read_write_paths(&u),
            ["/data/free space", "/var/lib/freespace"]
        );
    }

    #[test]
    fn calendars() {
        assert_eq!(on_calendar("daily", "02:00"), "*-*-* 02:00:00");
        assert_eq!(on_calendar("hourly", "02:15"), "*-*-* *:15:00");
        assert_eq!(on_calendar("weekly", "23:05"), "Mon *-*-* 23:05:00");
        let t = render_timer(&Config::default());
        assert!(t.contains("Persistent=true\n"));
        assert!(t.contains("WantedBy=timers.target\n"));
    }
}

//! Panic hook (D16/T39): one line on stderr naming the crash log, the details
//! in the log, and `last_error` in state.json when a scan or refresh crashed.

use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;

use clap::Parser;
use freespace_core::config;
use freespace_core::platform::now_secs;
use freespace_core::snapshot::{fsutil, state};

/// DX file-mode table: root writes into snapshot_dir (0640), users into
/// `$XDG_STATE_HOME/freespace/` (0600).
fn crash_dir() -> (PathBuf, u32) {
    if fsutil::is_root() {
        let dir = crate::backend::load()
            .map(|l| crate::backend::dir(&l).path)
            .unwrap_or_else(|_| PathBuf::from(config::DEFAULT_SNAPSHOT_DIR));
        return (dir, 0o640);
    }
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    (base.join("freespace"), 0o600)
}

pub fn install_hook() {
    std::panic::set_hook(Box::new(|info| {
        let msg = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "panic".into());
        let at = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_default();
        let version = freespace_core::VERSION;
        let args: Vec<std::ffi::OsString> = std::env::args_os().collect();
        let (dir, mode) = crash_dir();
        let path = dir.join(format!("crash-{}.log", now_secs()));
        let written = std::fs::create_dir_all(&dir).is_ok()
            && std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(mode)
                .open(&path)
                .and_then(|mut f| {
                    if fsutil::is_root() {
                        let _ = fsutil::secure_file(&f);
                    }
                    writeln!(
                        f,
                        "freespace {version} crashed: {msg}\nat: {at}\nargs: {args:?}\n\n{}",
                        std::backtrace::Backtrace::force_capture()
                    )
                })
                .is_ok();
        let trace = if written {
            path.display().to_string()
        } else {
            "not written".into()
        };
        eprintln!(
            "freespace {version} crashed: {msg}. trace: {trace}. rerun with RUST_BACKTRACE=1"
        );
        // a crashed scan or refresh holds the lock: record it for status/doctor
        if fsutil::is_root()
            && let Some(op) = operation(&args)
            && let Ok(l) = crate::backend::load()
        {
            state::record_error(&crate::backend::dir(&l), op, &format!("crashed: {msg}"));
        }
    }));
}

/// Reuse Clap's grammar so global option values or paths named `scan` cannot
/// be mistaken for the running command. OsString also preserves raw paths.
fn operation(args: &[std::ffi::OsString]) -> Option<&'static str> {
    let cli = crate::Cli::try_parse_from(args).ok()?;
    match cli.command {
        Some(crate::Command::Scan(_)) => Some("scan"),
        Some(crate::Command::Refresh(_)) => Some("refresh"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn detects_writer_commands_with_global_flags() {
        for args in [
            vec!["freespace", "scan"],
            vec!["freespace", "--backend", "v2", "scan", "/tmp"],
            vec!["freespace", "refresh", "--backend=v2", "/tmp"],
        ] {
            let args = args
                .into_iter()
                .map(std::ffi::OsString::from)
                .collect::<Vec<_>>();
            assert!(super::operation(&args).is_some());
        }
        for args in [
            vec!["freespace", "ls", "scan"],
            vec!["freespace", "--backend", "scan"],
            vec!["freespace", "status"],
        ] {
            let args = args
                .into_iter()
                .map(std::ffi::OsString::from)
                .collect::<Vec<_>>();
            assert_eq!(super::operation(&args), None);
        }
    }
}

//! Panic hook (D16/T39): one line on stderr naming the crash log, the details
//! in the log, and `last_error` in state.json when a scan or refresh crashed.

use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;

use freespace_core::config;
use freespace_core::platform::now_secs;
use freespace_core::snapshot::{SnapshotDir, fsutil, state};

/// DX file-mode table: root writes into snapshot_dir (0640), users into
/// `$XDG_STATE_HOME/freespace/` (0600).
fn crash_dir() -> (PathBuf, u32) {
    if fsutil::is_root() {
        let dir = config::load()
            .map(|l| l.config.snapshot_dir)
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
        let args: Vec<String> = std::env::args().collect();
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
            && let Some(op) = args.get(1).filter(|a| *a == "scan" || *a == "refresh")
            && let Ok(l) = config::load()
        {
            state::record_error(
                &SnapshotDir::new(l.config.snapshot_dir),
                op,
                &format!("crashed: {msg}"),
            );
        }
    }));
}

//! `gui`: start the server, hand the token to the browser through a one-shot
//! 0600 redirect file (never argv), or print the ssh tunnel line (DX D11).

use std::io::Write;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use clap::Args;
use freespace_core::error::{Error, ErrorCode, Result};
use freespace_core::platform;
use tokio::process::Command;

use super::{Server, runtime};
use crate::output::{stdin_is_tty, stdout_is_tty};

/// Seconds the redirect file may live when no browser consumes it.
const BOOTSTRAP_TTL: u64 = 60;
/// Print the URL when the page has not called the API by then.
const OPEN_GRACE: u64 = 10;

#[derive(Args)]
pub struct GuiArgs {
    /// Folder to open (default: the first scanned root)
    #[arg(value_hint = clap::ValueHint::DirPath)]
    pub path: Option<PathBuf>,
    /// Port on 127.0.0.1 (default: config `port`; 0 picks a free one)
    #[arg(long)]
    pub port: Option<u16>,
}

fn no_tty() -> Error {
    Error::new(ErrorCode::GuiNoTty, "gui needs a terminal and a browser").with_fix(
        "for data run freespace ls / --json; for a human: run freespace gui in their terminal \
         (over ssh: ssh -t -L 8710:127.0.0.1:8710 host freespace gui --port 8710)",
    )
}

/// `#/home/tass` fragment for a folder; bytes outside the unreserved set are
/// percent-encoded, non-UTF-8 paths travel as `?raw=<hex>`.
pub fn fragment(path: &Path) -> String {
    let b = path.as_os_str().as_bytes();
    if std::str::from_utf8(b).is_err() {
        let hex: String = b.iter().map(|x| format!("{x:02x}")).collect();
        return format!("/?raw={hex}");
    }
    let mut out = String::new();
    for &c in b {
        if c.is_ascii_alphanumeric() || b"/-._~".contains(&c) {
            out.push(c as char);
        } else {
            out.push_str(&format!("%{c:02X}"));
        }
    }
    out
}

fn over_ssh() -> bool {
    let set = |k: &str| std::env::var_os(k).is_some_and(|v| !v.is_empty());
    set("SSH_CONNECTION") && !set("DISPLAY") && !set("WAYLAND_DISPLAY")
}

/// `$XDG_RUNTIME_DIR`, else `~/.cache/freespace` (0700).
fn private_dir() -> Result<PathBuf> {
    if let Some(d) = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from)
        && d.is_dir()
    {
        return Ok(d);
    }
    let home = platform::current_user()
        .map(|u| u.home)
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
        .ok_or_else(|| Error::generic("no XDG_RUNTIME_DIR and no home directory"))?;
    let d = home.join(".cache/freespace");
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&d)
        .map_err(|e| Error::io(format!("cannot create {}", d.display()), e))?;
    Ok(d)
}

fn write_bootstrap(url: &str) -> Result<PathBuf> {
    let dir = private_dir()?;
    sweep_bootstraps(&dir, Duration::from_secs(BOOTSTRAP_TTL));
    write_bootstrap_in(&dir, url)
}

/// Remove redirect files a killed `gui` left behind (their tokens are dead,
/// but the name of the folder it opened is not).
fn sweep_bootstraps(dir: &Path, older_than: Duration) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let name = e.file_name();
        let n = name.to_string_lossy();
        let stale = e
            .metadata()
            .and_then(|m| m.modified())
            .is_ok_and(|t| t.elapsed().unwrap_or_default() > older_than);
        if n.starts_with("freespace-gui-") && n.ends_with(".html") && stale {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

fn write_bootstrap_in(dir: &Path, url: &str) -> Result<PathBuf> {
    let p = dir.join(format!("freespace-gui-{}.html", std::process::id()));
    let _ = std::fs::remove_file(&p);
    let html = format!(
        "<!doctype html><meta charset=\"utf-8\"><meta name=\"referrer\" content=\"no-referrer\">\
         <meta http-equiv=\"refresh\" content=\"0;url={url}\"><title>freespace</title>\
         <a href=\"{url}\">Open freespace</a>\n"
    );
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&p)
        .and_then(|mut f| f.write_all(html.as_bytes()))
        .map_err(|e| Error::io(format!("cannot write {}", p.display()), e))?;
    Ok(p)
}

/// `xdg-open <file>`; false when it is missing or fails. Async, so a browser
/// that keeps `xdg-open` in the foreground never holds up Ctrl-C.
async fn open_browser(file: &Path) -> bool {
    Command::new("xdg-open")
        .arg(file)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .is_ok_and(|s| s.success())
}

pub fn run(a: &GuiArgs) -> Result<()> {
    if !stdin_is_tty() && !stdout_is_tty() {
        return Err(no_tty());
    }
    let frag = match &a.path {
        Some(p) => {
            let abs = if p.is_absolute() {
                p.clone()
            } else {
                let cwd = std::env::current_dir().map_err(|e| Error::io("cannot read cwd", e))?;
                std::fs::canonicalize(cwd.join(p)).unwrap_or_else(|_| cwd.join(p))
            };
            fragment(&abs)
        }
        None => String::new(),
    };
    runtime()?.block_on(async move {
        let srv = Server::bind(a.port).await?;
        let url = srv.url(&frag);
        let port = srv.port;
        if over_ssh() {
            eprintln!(
                "freespace gui is running on {host}, port {port}. On your computer run:\n  \
                 ssh -N -L {port}:127.0.0.1:{port} {host}\nthen open:\n  {url}\npress Ctrl-C to stop",
                host = platform::hostname()
            );
            return srv.run(None).await;
        }
        let file = match write_bootstrap(&url) {
            Ok(f) => Some(f),
            Err(e) => {
                eprintln!("freespace: {}; open {url}", e.message);
                None
            }
        };
        eprintln!("freespace gui on http://127.0.0.1:{port}/ · press Ctrl-C to stop");
        if let Some(f) = file.clone() {
            let st = srv.state.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_secs(BOOTSTRAP_TTL)).await;
                st.consume_bootstrap();
            });
            let st = srv.state.clone();
            let url = url.clone();
            tokio::spawn(async move {
                let grace = Duration::from_secs(OPEN_GRACE);
                let start = tokio::time::Instant::now();
                // xdg-open may stay in the foreground with the browser: stop waiting
                match tokio::time::timeout(grace, open_browser(&f)).await {
                    Ok(false) => {
                        eprintln!("no browser opened; open {url}");
                        return;
                    }
                    Ok(true) => tokio::time::sleep_until(start + grace).await,
                    Err(_) => {}
                }
                if st.bootstrap_pending() {
                    eprintln!("the page has not loaded yet; open {url}");
                }
            });
        }
        srv.run(file).await
    })
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::os::unix::fs::PermissionsExt;

    use super::*;
    use crate::serve::page_url;

    #[test]
    fn fragments_round_trip_through_the_page_router() {
        assert_eq!(fragment(Path::new("/home/tass")), "/home/tass");
        assert_eq!(fragment(Path::new("/a b/ü#?%")), "/a%20b/%C3%BC%23%3F%25");
        let raw = Path::new(OsStr::from_bytes(b"/x/\xff"));
        assert_eq!(fragment(raw), "/?raw=2f782fff");
    }

    #[test]
    fn the_token_joins_the_fragment_once() {
        assert_eq!(page_url(8710, "", "ab"), "http://127.0.0.1:8710/#t=ab");
        assert_eq!(
            page_url(8710, "/home", "ab"),
            "http://127.0.0.1:8710/#/home?t=ab"
        );
        assert_eq!(
            page_url(1, "/?raw=ff", "ab"),
            "http://127.0.0.1:1/#/?raw=ff&t=ab"
        );
    }

    #[test]
    fn bootstrap_file_is_private_and_carries_the_url() {
        let d = tempfile::tempdir().unwrap();
        let url = page_url(8710, "/home", "secret");
        let p = write_bootstrap_in(d.path(), &url).unwrap();
        let m = std::fs::metadata(&p).unwrap();
        assert_eq!(m.permissions().mode() & 0o777, 0o600);
        let html = std::fs::read_to_string(&p).unwrap();
        assert!(html.contains(&format!("content=\"0;url={url}\"")), "{html}");
        assert!(html.contains("no-referrer"));
        // a stale file of the same name is replaced, not followed
        assert!(write_bootstrap_in(d.path(), &url).is_ok());
    }

    #[test]
    fn leftover_redirect_files_are_swept() {
        let d = tempfile::tempdir().unwrap();
        let old = d.path().join("freespace-gui-1.html");
        let other = d.path().join("notes.html");
        std::fs::write(&old, "x").unwrap();
        std::fs::write(&other, "x").unwrap();
        sweep_bootstraps(d.path(), Duration::from_secs(60));
        assert!(old.exists(), "a fresh file may still be in use");
        sweep_bootstraps(d.path(), Duration::ZERO);
        assert!(!old.exists());
        assert!(other.exists());
    }
}

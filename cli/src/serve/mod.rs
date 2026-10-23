//! `serve` and `gui`: the page and `/api/*` on 127.0.0.1 only, a fresh token per
//! process, until Ctrl-C (T4, eng D6/D8, DX D11/D15).

mod api;
mod assets;
mod gui;
mod pool;

use std::future::Future;
use std::io::Read;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use clap::Args;
use freespace_core::config;
use freespace_core::error::{Error, Result};
use tokio::net::TcpListener;

pub use gui::{GuiArgs, run as gui};

#[derive(Args)]
pub struct ServeArgs {
    /// Port on 127.0.0.1 (default: config `port`; 0 picks a free one)
    #[arg(long)]
    pub port: Option<u16>,
}

/// A bound listener with its token, ready to run.
pub struct Server {
    listener: TcpListener,
    pub port: u16,
    state: Arc<api::AppState>,
    /// Signal handlers exist from `bind` on: a Ctrl-C right after the URL is
    /// printed still stops cleanly and removes the bootstrap file.
    stop: Pin<Box<dyn Future<Output = ()> + Send>>,
}

fn runtime() -> Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| Error::io("cannot start the server runtime", e))
}

/// `http://127.0.0.1:<port>/#<fragment>` with the token in the fragment
/// (never sent to a server; the page keeps it, OV-14).
pub fn page_url(port: u16, fragment: &str, token: &str) -> String {
    let sep = if fragment.is_empty() {
        ""
    } else if fragment.contains('?') {
        "&"
    } else {
        "?"
    };
    format!("http://127.0.0.1:{port}/#{fragment}{sep}t={token}")
}

fn new_token() -> Result<String> {
    let mut b = [0u8; 32];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut b))
        .map_err(|e| Error::io("cannot read /dev/urandom", e))?;
    Ok(b.iter().map(|x| format!("{x:02x}")).collect())
}

impl Server {
    pub async fn bind(port: Option<u16>) -> Result<Server> {
        let loaded = config::load()?;
        let port = port.unwrap_or(loaded.config.port);
        let listener = TcpListener::bind(("127.0.0.1", port)).await.map_err(|e| {
            let mut err = Error::io(format!("cannot listen on 127.0.0.1:{port}"), e);
            err = err.with_fix("pass --port 0 for a free port, or stop the other freespace gui");
            err
        })?;
        let port = listener
            .local_addr()
            .map_err(|e| Error::io("cannot read the listening port", e))?
            .port();
        let dir = loaded.snapshot_dir();
        let state = Arc::new(api::AppState {
            port,
            token: new_token()?,
            pool: pool::Pool::new(dir.clone()),
            dir,
            config: loaded.path.display().to_string(),
            bootstrap: Mutex::new(None),
        });
        Ok(Server {
            listener,
            port,
            state,
            stop: Box::pin(shutdown()?),
        })
    }

    pub fn url(&self, path_fragment: &str) -> String {
        page_url(self.port, path_fragment, &self.state.token)
    }

    /// Serve until Ctrl-C, SIGTERM or SIGHUP.
    pub async fn run(self, bootstrap: Option<PathBuf>) -> Result<()> {
        if let Ok(mut g) = self.state.bootstrap.lock() {
            *g = bootstrap;
        }
        let st = self.state.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(1));
            loop {
                tick.tick().await;
                st.pool.reap();
            }
        });
        let app = api::router(self.state.clone());
        let res = axum::serve(self.listener, app)
            .with_graceful_shutdown(self.stop)
            .await
            .map_err(|e| Error::io("server failed", e));
        self.state.consume_bootstrap();
        res
    }
}

/// Ctrl-C, SIGTERM or SIGHUP, registered now (D8 = B: run until stopped).
fn shutdown() -> Result<impl Future<Output = ()> + Send + 'static> {
    use tokio::signal::unix::{SignalKind, signal};
    let on = |k: SignalKind| signal(k).map_err(|e| Error::io("cannot install signal handlers", e));
    let (mut int, mut term, mut hup) = (
        on(SignalKind::interrupt())?,
        on(SignalKind::terminate())?,
        on(SignalKind::hangup())?,
    );
    Ok(async move {
        tokio::select! {
            _ = int.recv() => {}
            _ = term.recv() => {}
            _ = hup.recv() => {}
        }
    })
}

pub fn run(a: &ServeArgs) -> Result<()> {
    runtime()?.block_on(async {
        let srv = Server::bind(a.port).await?;
        eprintln!("freespace serve: {}\npress Ctrl-C to stop", srv.url(""));
        srv.run(None).await
    })
}

#[cfg(test)]
mod tests;

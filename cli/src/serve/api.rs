//! Router, request guard and `/api/*` handlers. Parameters and JSON are the
//! CLI's one to one: each handler calls the same `core::queries` function.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Body;
use axum::extract::{RawQuery, Request, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get};
use freespace_core::error::{Error, ErrorCode, Result};
use freespace_core::queries::{self, params};
use freespace_core::snapshot::SnapshotDir;
use freespace_core::snapshot::reader::Snapshot;
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::assets;
use super::pool::Pool;

pub const TOKEN_HEADER: &str = "x-freespace-token";

const CSP: &str = "default-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";

pub struct AppState {
    pub port: u16,
    pub backend: freespace_core::config::Backend,
    pub token: String,
    pub dir: SnapshotDir,
    pub config: String,
    pub pool: Pool,
    /// The one-shot redirect file written by `gui`; removed on the first
    /// authorized API request or after its TTL.
    pub bootstrap: Mutex<Option<PathBuf>>,
}

impl AppState {
    fn host_ok(&self, host: &str) -> bool {
        host == format!("127.0.0.1:{}", self.port) || host == format!("localhost:{}", self.port)
    }

    fn origin_ok(&self, origin: &str) -> bool {
        origin == format!("http://127.0.0.1:{}", self.port)
            || origin == format!("http://localhost:{}", self.port)
    }

    fn token_ok(&self, got: &[u8]) -> bool {
        let want = self.token.as_bytes();
        got.len() == want.len() && got.iter().zip(want).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0
    }

    pub fn bootstrap_pending(&self) -> bool {
        self.bootstrap.lock().map(|g| g.is_some()).unwrap_or(false)
    }

    pub fn consume_bootstrap(&self) {
        let taken = self.bootstrap.lock().ok().and_then(|mut g| g.take());
        if let Some(p) = taken {
            let _ = std::fs::remove_file(p);
        }
    }

    fn error(&self, e: &Error) -> Response {
        let dir = self.dir.path.display().to_string();
        let body = e.body(Some(&dir), Some(&self.config));
        let status =
            StatusCode::from_u16(e.code.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        json(status, &body)
    }

    fn respond<T: Serialize>(
        &self,
        r: std::result::Result<Result<T>, tokio::task::JoinError>,
    ) -> Response {
        match r {
            Ok(Ok(v)) => json(StatusCode::OK, &v),
            Ok(Err(e)) => self.error(&e),
            Err(_) => self.error(&Error::new(
                ErrorCode::Generic,
                "the query failed unexpectedly",
            )),
        }
    }

    /// Run `f` on a pooled connection off the runtime thread.
    async fn with_snapshot<T, F>(self: &Arc<Self>, f: F) -> Response
    where
        T: Serialize + Send + 'static,
        F: FnOnce(&Snapshot) -> Result<T> + Send + 'static,
    {
        let st = self.clone();
        let r = tokio::task::spawn_blocking(move || {
            let (snap, key) = st.pool.get()?;
            let out = f(&snap);
            st.pool.put(snap, key);
            out
        })
        .await;
        self.respond(r)
    }

    async fn with_v2<T, F>(self: &Arc<Self>, f: F) -> Response
    where
        T: Serialize + Send + 'static,
        F: FnOnce(&freespace_core::snapshot::v2::Reader) -> Result<T> + Send + 'static,
    {
        let dir = self.dir.clone();
        let result = tokio::task::spawn_blocking(move || f(&crate::backend::reader(&dir)?)).await;
        self.respond(result)
    }

    async fn with_dir<T, F>(self: &Arc<Self>, f: F) -> Response
    where
        T: Serialize + Send + 'static,
        F: FnOnce(&SnapshotDir) -> Result<T> + Send + 'static,
    {
        let dir = self.dir.clone();
        let r = tokio::task::spawn_blocking(move || f(&dir)).await;
        self.respond(r)
    }
}

fn json<T: Serialize>(status: StatusCode, v: &T) -> Response {
    match serde_json::to_vec(v) {
        Ok(b) => (status, [(header::CONTENT_TYPE, "application/json")], b).into_response(),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "serialization failed").into_response(),
    }
}

fn parse<T: DeserializeOwned>(q: Option<String>) -> Result<T> {
    serde_urlencoded::from_str(q.as_deref().unwrap_or(""))
        .map_err(|e| Error::usage(format!("bad query parameter: {e}")))
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/status", get(status))
        .route("/api/ls", get(ls))
        .route("/api/top", get(top))
        .route("/api/find", get(find))
        .route("/api/diff", get(diff))
        .route("/api/{*rest}", any(unknown))
        .fallback(assets::serve)
        .layer(middleware::from_fn_with_state(state.clone(), guard))
        .with_state(state)
}

fn plain(status: StatusCode, text: String) -> Response {
    (
        status,
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        text,
    )
        .into_response()
}

/// Host check for everything; Origin, Sec-Fetch-Site and the token for `/api/*`;
/// security headers on every answer. 403s carry no detail beyond the port hint.
async fn guard(State(s): State<Arc<AppState>>, req: Request, next: Next) -> Response {
    let resp = match check(&s, req.method(), req.uri().path(), req.headers()) {
        Some(r) => r,
        None => next.run(req).await,
    };
    secure(resp)
}

fn check(s: &AppState, method: &Method, path: &str, h: &HeaderMap) -> Option<Response> {
    if method != Method::GET && method != Method::HEAD {
        return Some(plain(StatusCode::METHOD_NOT_ALLOWED, "only GET".into()));
    }
    let host = h
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !s.host_ok(host) {
        let p = s.port;
        return Some(plain(
            StatusCode::FORBIDDEN,
            format!(
                "freespace answers on 127.0.0.1:{p} only. Open http://127.0.0.1:{p}/ ; an ssh tunnel must keep the port: ssh -N -L {p}:127.0.0.1:{p} <host>\n"
            ),
        ));
    }
    if !path.starts_with("/api/") {
        return None;
    }
    let forbidden = || Some(plain(StatusCode::FORBIDDEN, "forbidden\n".into()));
    if let Some(o) = h.get(header::ORIGIN)
        && !o.to_str().map(|o| s.origin_ok(o)).unwrap_or(false)
    {
        return forbidden();
    }
    if h.get("sec-fetch-site")
        .is_some_and(|v| v.as_bytes() == b"cross-site" || v.as_bytes() == b"same-site")
    {
        return forbidden();
    }
    let token = h.get(TOKEN_HEADER).map(|v| v.as_bytes()).unwrap_or(b"");
    if !s.token_ok(token) {
        return forbidden();
    }
    s.consume_bootstrap();
    None
}

fn secure(mut r: Response) -> Response {
    let h = r.headers_mut();
    let set = |h: &mut HeaderMap, k: &'static str, v: &'static str| {
        h.insert(k, HeaderValue::from_static(v));
    };
    set(h, "content-security-policy", CSP);
    set(h, "referrer-policy", "no-referrer");
    set(h, "x-content-type-options", "nosniff");
    set(h, "x-frame-options", "DENY");
    set(h, "cross-origin-resource-policy", "same-origin");
    if !h.contains_key(header::CACHE_CONTROL) {
        set(h, "cache-control", "no-store");
    }
    r
}

async fn unknown(State(s): State<Arc<AppState>>, req: Request<Body>) -> Response {
    let mut r = s.error(&Error::usage(format!(
        "unknown endpoint {}: use /api/status, ls, top, find or diff",
        req.uri().path()
    )));
    *r.status_mut() = StatusCode::NOT_FOUND;
    r
}

async fn status(State(s): State<Arc<AppState>>) -> Response {
    let backend = s.backend;
    s.with_dir(move |dir| crate::backend::status(backend, dir, true))
        .await
}

async fn ls(State(s): State<Arc<AppState>>, RawQuery(q): RawQuery) -> Response {
    match parse::<params::LsParams>(q) {
        Ok(p) if s.backend == freespace_core::config::Backend::V2 => {
            s.with_v2(move |reader| queries::v2::ls(reader, &p)).await
        }
        Ok(p) => s.with_snapshot(move |snap| queries::ls(snap, &p)).await,
        Err(e) => s.error(&e),
    }
}

async fn top(State(s): State<Arc<AppState>>, RawQuery(q): RawQuery) -> Response {
    match parse::<params::TopParams>(q) {
        Ok(p) if s.backend == freespace_core::config::Backend::V2 => {
            s.with_v2(move |reader| queries::v2::top(reader, &p)).await
        }
        Ok(p) => s.with_snapshot(move |snap| queries::top(snap, &p)).await,
        Err(e) => s.error(&e),
    }
}

async fn find(State(s): State<Arc<AppState>>, RawQuery(q): RawQuery) -> Response {
    match parse::<params::FindParams>(q) {
        Ok(p) if s.backend == freespace_core::config::Backend::V2 => {
            s.with_v2(move |reader| queries::v2::find(reader, &p)).await
        }
        Ok(p) => s.with_snapshot(move |snap| queries::find(snap, &p)).await,
        Err(e) => s.error(&e),
    }
}

async fn diff(State(s): State<Arc<AppState>>, RawQuery(q): RawQuery) -> Response {
    match parse::<params::DiffParams>(q) {
        Ok(p) if s.backend == freespace_core::config::Backend::V2 => {
            s.with_v2(move |reader| queries::v2::diff(reader, &p)).await
        }
        Ok(p) => s.with_dir(move |d| queries::diff(d, &p)).await,
        Err(e) => s.error(&e),
    }
}

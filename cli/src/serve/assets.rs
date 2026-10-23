//! The page: bundled by `bun build` in `build.rs`, embedded from `OUT_DIR`.

use axum::http::{StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use include_dir::{Dir, include_dir};

static WEB: Dir<'static> = include_dir!("$OUT_DIR/web");

fn content_type(path: &str) -> &'static str {
    match path.rsplit_once('.').map(|(_, e)| e) {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("woff2") => "font/woff2",
        Some("svg") => "image/svg+xml",
        Some("txt") => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

pub fn get(path: &str) -> Option<&'static [u8]> {
    WEB.get_file(path).map(|f| f.contents())
}

/// Static files need no token (the Host check still applies).
pub async fn serve(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };
    let Some(body) = get(path) else {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    };
    // Fonts carry a content hash in their name (build.rs).
    let cache = if path.ends_with(".woff2") {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    (
        [
            (header::CONTENT_TYPE, content_type(path)),
            (header::CACHE_CONTROL, cache),
        ],
        body,
    )
        .into_response()
}

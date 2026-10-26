//! The server over raw TCP (T4): Host, token, Origin and Sec-Fetch-Site
//! checks, security headers, static files without a token, error bodies,
//! the bootstrap file and pool rotation. Raw sockets so the tests can send
//! the headers a browser never would.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use freespace_core::job::{self, ScanRequest};
use freespace_core::queries::{self, params::LsParams};
use freespace_core::snapshot::SnapshotDir;
use freespace_core::snapshot::reader::Snapshot;
use serde_json::Value;

use super::api::{self, AppState};
use super::pool::Pool;

const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

struct Fx {
    tmp: tempfile::TempDir,
    root: PathBuf,
    dir: SnapshotDir,
    state: Arc<AppState>,
    port: u16,
}

fn scan(root: &Path, dir: &SnapshotDir, keep_previous: bool) {
    job::run_scan(ScanRequest {
        roots: vec![root.to_path_buf()],
        low_memory: false,
        one_file_system: true,
        workers: 2,
        dir: dir.clone(),
        keep_previous,
        on_progress: None,
    })
    .unwrap();
}

fn start(with_snapshot: bool) -> Fx {
    start_backend(with_snapshot, freespace_core::config::Backend::V1)
}

fn start_backend(with_snapshot: bool, backend: freespace_core::config::Backend) -> Fx {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("r");
    std::fs::create_dir_all(root.join("sub")).unwrap();
    std::fs::write(root.join("sub/a.bin"), vec![1u8; 64 * 1024]).unwrap();
    let dir = SnapshotDir::new(tmp.path().join("snap"));
    if with_snapshot {
        if backend == freespace_core::config::Backend::V1 {
            scan(&root, &dir, false);
        } else {
            std::fs::create_dir_all(dir.as_path()).unwrap();
            freespace_core::snapshot::v2::Store::open(&crate::backend::database(&dir))
                .unwrap()
                .scan(&root, 2)
                .unwrap();
        }
    }
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();
    let state = Arc::new(AppState {
        backend,
        port,
        token: TOKEN.into(),
        pool: Pool::new(dir.clone()),
        dir: dir.clone(),
        config: "/etc/freespace/config.toml".into(),
        bootstrap: Mutex::new(None),
    });
    let st = state.clone();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async move {
            let l = tokio::net::TcpListener::from_std(listener).unwrap();
            axum::serve(l, api::router(st)).await.unwrap();
        });
    });
    Fx {
        tmp,
        root,
        dir,
        state,
        port,
    }
}

struct Resp {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Resp {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    fn json(&self) -> Value {
        assert_eq!(
            self.header("content-type"),
            Some("application/json"),
            "{}",
            self.text()
        );
        serde_json::from_slice(&self.body).unwrap()
    }
}

impl Fx {
    fn host(&self) -> String {
        format!("127.0.0.1:{}", self.port)
    }

    /// One request; `headers` replace the default Host when they name one.
    fn send(&self, method: &str, path: &str, headers: &[(&str, &str)]) -> Resp {
        let mut req = format!("{method} {path} HTTP/1.1\r\nConnection: close\r\n");
        if !headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("host")) {
            req.push_str(&format!("Host: {}\r\n", self.host()));
        }
        for (k, v) in headers {
            req.push_str(&format!("{k}: {v}\r\n"));
        }
        req.push_str("\r\n");
        let mut s = TcpStream::connect(("127.0.0.1", self.port)).unwrap();
        s.write_all(req.as_bytes()).unwrap();
        let mut raw = Vec::new();
        s.read_to_end(&mut raw).unwrap();
        let split = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
        let head = String::from_utf8(raw[..split].to_vec()).unwrap();
        let mut lines = head.split("\r\n");
        let status = lines
            .next()
            .unwrap()
            .split(' ')
            .nth(1)
            .unwrap()
            .parse()
            .unwrap();
        let headers = lines
            .filter_map(|l| l.split_once(':'))
            .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_owned()))
            .collect();
        Resp {
            status,
            headers,
            body: raw[split + 4..].to_vec(),
        }
    }

    fn get(&self, path: &str, headers: &[(&str, &str)]) -> Resp {
        self.send("GET", path, headers)
    }

    fn api(&self, path: &str) -> Resp {
        self.get(path, &[("X-Freespace-Token", TOKEN)])
    }

    fn q(&self, rel: &str) -> String {
        let p = if rel.is_empty() {
            self.root.clone()
        } else {
            self.root.join(rel)
        };
        p.to_str().unwrap().replace('/', "%2F")
    }
}

#[test]
fn host_must_name_our_port() {
    let fx = start(true);
    for bad in [
        "evil.example",
        "127.0.0.1",
        "127.0.0.1:1",
        "evil.example:80",
    ] {
        let r = fx.get(
            "/api/status",
            &[("Host", bad), ("X-Freespace-Token", TOKEN)],
        );
        assert_eq!(r.status, 403, "{bad}");
        assert!(
            r.text().contains(&format!("127.0.0.1:{}", fx.port)),
            "{}",
            r.text()
        );
        assert_eq!(fx.get("/", &[("Host", bad)]).status, 403, "static {bad}");
    }
    let local = format!("localhost:{}", fx.port);
    let r = fx.get(
        "/api/status",
        &[("Host", &local), ("X-Freespace-Token", TOKEN)],
    );
    assert_eq!(r.status, 200);
}

#[test]
fn api_needs_the_token_and_static_files_do_not() {
    let fx = start(true);
    let wrong = TOKEN.replace('0', "f");
    for h in [
        vec![],
        vec![("X-Freespace-Token", wrong.as_str())],
        vec![("X-Freespace-Token", "short")],
    ] {
        let r = fx.get("/api/status", &h);
        assert_eq!(r.status, 403);
        // no JSON, no hint: a page on another origin learns nothing
        assert_eq!(r.text(), "forbidden\n");
    }
    // the token in the query string is not a token
    assert_eq!(fx.get(&format!("/api/status?t={TOKEN}"), &[]).status, 403);
    let r = fx.api("/api/status");
    assert_eq!(r.status, 200);
    assert_eq!(r.json()["generation"], 1);

    let page = fx.get("/", &[]);
    assert_eq!(page.status, 200);
    assert!(page.text().contains("<div id=\"app\">"));
    assert_eq!(fx.get("/main.js", &[]).status, 200);
    assert_eq!(fx.get("/main.css", &[]).status, 200);
    assert_eq!(fx.get("/nope.js", &[]).status, 404);
}

#[test]
fn origin_and_fetch_site_are_checked() {
    let fx = start(true);
    let ours = format!("http://127.0.0.1:{}", fx.port);
    let cases: [(&str, &str, u16); 7] = [
        ("Origin", "http://evil.example", 403),
        ("Origin", "http://127.0.0.1:1", 403),
        ("Origin", "null", 403),
        ("Origin", &ours, 200),
        ("Sec-Fetch-Site", "cross-site", 403),
        ("Sec-Fetch-Site", "same-site", 403),
        ("Sec-Fetch-Site", "same-origin", 200),
    ];
    for (k, v, want) in cases {
        let r = fx.get("/api/status", &[("X-Freespace-Token", TOKEN), (k, v)]);
        assert_eq!(r.status, want, "{k}: {v}");
    }
}

#[test]
fn only_get_and_head() {
    let fx = start(true);
    for m in ["POST", "PUT", "DELETE", "OPTIONS"] {
        let r = fx.send(
            m,
            "/api/status",
            &[("X-Freespace-Token", TOKEN), ("Content-Length", "0")],
        );
        assert_eq!(r.status, 405, "{m}");
    }
    assert_eq!(fx.send("HEAD", "/", &[]).status, 200);
}

#[test]
fn security_and_cache_headers() {
    let fx = start(true);
    let page = fx.get("/", &[]);
    let api = fx.api("/api/status");
    let denied = fx.get("/api/status", &[]);
    for r in [&page, &api, &denied] {
        assert_eq!(
            r.header("content-security-policy"),
            Some("default-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'")
        );
        assert_eq!(r.header("referrer-policy"), Some("no-referrer"));
        assert_eq!(r.header("x-content-type-options"), Some("nosniff"));
        assert_eq!(r.header("x-frame-options"), Some("DENY"));
        assert_eq!(
            r.header("cross-origin-resource-policy"),
            Some("same-origin")
        );
    }
    assert_eq!(page.header("cache-control"), Some("no-cache"));
    assert_eq!(
        page.header("content-type"),
        Some("text/html; charset=utf-8")
    );
    assert_eq!(api.header("cache-control"), Some("no-store"));

    let css = fx.get("/fonts/fonts.css", &[]);
    assert_eq!(css.status, 200);
    let css = css.text();
    let start = css.find("url(\"").unwrap() + 5;
    let name = &css[start..start + css[start..].find('"').unwrap()];
    assert!(
        name.ends_with(".woff2") && name.matches('-').count() >= 4,
        "{name}"
    );
    let font = fx.get(&format!("/fonts/{name}"), &[]);
    assert_eq!(font.status, 200);
    assert_eq!(font.header("content-type"), Some("font/woff2"));
    assert_eq!(
        font.header("cache-control"),
        Some("public, max-age=31536000, immutable")
    );
}

#[test]
fn errors_use_the_error_body_and_http_status() {
    let fx = start(true);
    let root = fx.q("");
    let cases = [
        ("/api/nope".to_owned(), 404, "USAGE"),
        (
            "/api/ls?path=%2Fno%2Fsuch%2Fpath".into(),
            404,
            "PATH_NOT_FOUND",
        ),
        (format!("/api/ls?path={root}&limit=0"), 400, "USAGE"),
        (format!("/api/ls?path={root}&limit=abc"), 400, "USAGE"),
        (format!("/api/ls?path={root}&path_raw=2f"), 400, "USAGE"),
        (format!("/api/top?path={root}&min_size=12X"), 400, "USAGE"),
        (
            format!("/api/find?path={root}&older_than=yesterday"),
            400,
            "USAGE",
        ),
        (
            format!("/api/ls?path={root}&snapshot=other"),
            409,
            "ROTATED",
        ),
        (format!("/api/diff?path={root}"), 404, "NO_PREV"),
    ];
    for (path, status, code) in cases {
        let r = fx.api(&path);
        assert_eq!(r.status, status, "{path}: {}", r.text());
        let v = r.json();
        assert_eq!(v["error"]["code"], code, "{path}");
        assert!(v["error"]["exit"].is_number());
        assert_eq!(
            v["error"]["context"]["config"],
            "/etc/freespace/config.toml"
        );
    }
}

#[test]
fn before_the_first_scan_status_answers_and_queries_say_no_snapshot() {
    let fx = start(false);
    let r = fx.api("/api/status");
    assert_eq!(r.status, 200);
    let v = r.json();
    assert!(v["generation"].is_null());
    assert!(v["snapshot_id"].is_null());
    let r = fx.api("/api/ls");
    assert_eq!(r.status, 404);
    assert_eq!(r.json()["error"]["code"], "NO_SNAPSHOT");
}

#[test]
fn answers_are_the_cli_json() {
    let fx = start(true);
    let r = fx.api(&format!("/api/ls?path={}&limit=5", fx.q("")));
    assert_eq!(r.status, 200);
    let got = r.json();
    let want = queries::ls(
        &Snapshot::open_current(&fx.dir).unwrap(),
        &LsParams {
            path: Some(fx.root.to_str().unwrap().into()),
            limit: Some(5),
            ..Default::default()
        },
    )
    .unwrap();
    let want = serde_json::to_value(want).unwrap();
    for k in [
        "entry",
        "entries",
        "total_count",
        "snapshot_id",
        "generation",
        "sort",
        "limit",
    ] {
        assert_eq!(got[k], want[k], "{k}");
    }
    assert_eq!(got["entries"][0]["name"], "sub");
}

#[test]
fn bootstrap_file_goes_on_the_first_authorized_request() {
    let fx = start(true);
    let file = fx.tmp.path().join("freespace-gui.html");
    std::fs::write(&file, "x").unwrap();
    *fx.state.bootstrap.lock().unwrap() = Some(file.clone());
    fx.get("/", &[]);
    fx.get("/api/status", &[]);
    fx.get(
        "/api/status",
        &[("X-Freespace-Token", TOKEN), ("Host", "evil.example")],
    );
    assert!(file.exists(), "unauthorized requests must not consume it");
    assert!(fx.state.bootstrap_pending());
    assert_eq!(fx.api("/api/status").status, 200);
    assert!(!file.exists());
    assert!(!fx.state.bootstrap_pending());
}

#[test]
fn pool_follows_the_published_snapshot() {
    let fx = start(true);
    let path = format!("/api/ls?path={}", fx.q(""));
    let first = fx.api(&path).json();
    let old = first["snapshot_id"].as_str().unwrap().to_owned();
    assert_eq!(
        fx.state.pool.idle(),
        1,
        "the connection goes back to the pool"
    );
    fx.api(&path);
    assert_eq!(fx.state.pool.idle(), 1, "and is reused");

    std::fs::write(fx.root.join("sub/b.bin"), vec![2u8; 32 * 1024]).unwrap();
    scan(&fx.root, &fx.dir, true);

    let r = fx.api(&format!("{path}&snapshot={old}"));
    assert_eq!(r.status, 409);
    assert_eq!(r.json()["error"]["code"], "ROTATED");
    let now = fx.api(&path).json();
    assert_ne!(now["snapshot_id"], first["snapshot_id"]);
    assert_eq!(now["generation"], 2);
    assert_eq!(fx.state.pool.idle(), 1, "old connections are dropped");
    let d = fx.api(&format!("/api/diff?path={}&kind=any", fx.q("sub")));
    assert_eq!(d.status, 200, "{}", d.text());
    assert_eq!(d.json()["entries"][0]["name"], "b.bin");
}

#[test]
fn v2_api_routes_every_endpoint_to_a_fresh_pin() {
    use freespace_core::queries::params::{DiffParams, FindParams, TopParams};
    let f = start_backend(true, freespace_core::config::Backend::V2);
    let mut store =
        freespace_core::snapshot::v2::Store::open(&crate::backend::database(&f.dir)).unwrap();
    store.scan(&f.root, 2).unwrap();
    std::fs::write(f.root.join("sub/b"), vec![4u8; 8192]).unwrap();
    store.refresh(&f.root.join("sub"), 2).unwrap();
    let path = f.root.to_str().unwrap().to_owned();
    let reader = crate::backend::reader(&f.dir).unwrap();
    let pairs = [
        (
            "ls",
            serde_json::to_value(
                queries::v2::ls(
                    &reader,
                    &LsParams {
                        path: Some(path.clone()),
                        ..Default::default()
                    },
                )
                .unwrap(),
            )
            .unwrap(),
        ),
        (
            "top",
            serde_json::to_value(
                queries::v2::top(
                    &reader,
                    &TopParams {
                        path: Some(path.clone()),
                        ..Default::default()
                    },
                )
                .unwrap(),
            )
            .unwrap(),
        ),
        (
            "find",
            serde_json::to_value(
                queries::v2::find(
                    &reader,
                    &FindParams {
                        path: Some(path.clone()),
                        ..Default::default()
                    },
                )
                .unwrap(),
            )
            .unwrap(),
        ),
        (
            "diff",
            serde_json::to_value(
                queries::v2::diff(
                    &reader,
                    &DiffParams {
                        path: Some(path),
                        ..Default::default()
                    },
                )
                .unwrap(),
            )
            .unwrap(),
        ),
    ];
    for (endpoint, mut expected) in pairs {
        let response = f.api(&format!("/api/{endpoint}?path={}", f.q("")));
        assert_eq!(response.status, 200, "{endpoint}: {:?}", response.body);
        let mut got = response.json();
        // Age may cross a wall-clock second between the two reads.
        got.as_object_mut().unwrap().remove("age_seconds");
        expected.as_object_mut().unwrap().remove("age_seconds");
        assert_eq!(got, expected, "{endpoint}");
    }
    assert_eq!(f.api("/api/status").json()["generation"], 3);
    assert_eq!(f.api("/api/status").json()["prev_generation"], 1);
    store.scan(&f.root, 2).unwrap();
    assert_eq!(f.api("/api/status").json()["generation"], 4);
}

#[test]
fn v2_api_missing_store_preserves_first_run_contract() {
    let f = start_backend(false, freespace_core::config::Backend::V2);
    assert_eq!(f.api("/api/status").status, 200);
    assert_eq!(f.api("/api/status").json()["generation"], Value::Null);
    assert_eq!(
        f.api("/api/ls").status,
        freespace_core::ErrorCode::NoSnapshot.http_status()
    );
    assert!(!crate::backend::database(&f.dir).exists());
}

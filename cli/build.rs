//! Builds the page with bun (eng D5): `web/src/main.ts` → `$OUT_DIR/web`, plus
//! `web/index.html`; `serve` embeds the result with `include_dir!`.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::Command;

const BUN_MISSING: &str = "freespace builds its web page with bun >= 1.1, and bun was not found. \
Install it: curl -fsSL https://bun.sh/install | bash (https://bun.sh), or point BUN at the binary.";

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let web = manifest.join("../web");
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR")).join("web");
    for p in ["src", "index.html", "fonts", "favicon.svg"] {
        println!("cargo:rerun-if-changed={}", web.join(p).display());
    }
    println!("cargo:rerun-if-env-changed=BUN");
    let bun = std::env::var("BUN").unwrap_or_else(|_| "bun".into());
    match Command::new(&bun).arg("--version").output() {
        Ok(o) if o.status.success() => {}
        _ => panic!("{BUN_MISSING}"),
    }
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).expect("create OUT_DIR/web");
    let status = Command::new(&bun)
        .arg("build")
        .arg(web.join("src/main.ts"))
        .arg("--outdir")
        .arg(&out)
        .args([
            "--minify",
            "--target=browser",
            "--entry-naming=[name].[ext]",
            "--asset-naming=[name]-[hash].[ext]",
        ])
        .status()
        .unwrap_or_else(|e| panic!("cannot run {bun} build: {e}"));
    assert!(status.success(), "bun build failed for {}", web.display());
    for f in ["index.html", "favicon.svg"] {
        copy(&web.join(f), &out.join(f));
    }
    fonts(&web.join("fonts"), &out.join("fonts"));
}

/// Fonts bypass bun, whose CSS bundler inlines every `url()` as a data URL
/// (+45 % CSS on each load). Each file gets a content hash in its name so
/// `serve` can cache it as immutable; `fonts.css` is rewritten to match.
fn fonts(src: &Path, out: &Path) {
    std::fs::create_dir_all(out).expect("create OUT_DIR/web/fonts");
    let read = |p: &Path| std::fs::read(p).unwrap_or_else(|e| panic!("read {}: {e}", p.display()));
    let mut css = String::from_utf8(read(&src.join("fonts.css"))).expect("fonts.css is UTF-8");
    let mut entries: Vec<PathBuf> = std::fs::read_dir(src)
        .expect("read web/fonts")
        .map(|e| e.expect("read web/fonts").path())
        .filter(|p| p.extension().is_some_and(|e| e == "woff2"))
        .collect();
    entries.sort();
    for p in entries {
        let bytes = read(&p);
        let mut h = DefaultHasher::new();
        bytes.hash(&mut h);
        let stem = p
            .file_stem()
            .and_then(|s| s.to_str())
            .expect("font name is UTF-8");
        let name = format!("{stem}-{:08x}.woff2", h.finish() as u32);
        let quoted = format!("\"{stem}.woff2\"");
        assert!(
            css.contains(&quoted),
            "fonts.css does not reference {stem}.woff2"
        );
        css = css.replace(&quoted, &format!("\"{name}\""));
        std::fs::write(out.join(&name), &bytes).expect("write font");
    }
    std::fs::write(out.join("fonts.css"), css).expect("write fonts.css");
    copy(&src.join("OFL.txt"), &out.join("OFL.txt"));
}

fn copy(from: &Path, to: &Path) {
    std::fs::copy(from, to).unwrap_or_else(|e| panic!("copy {}: {e}", from.display()));
}

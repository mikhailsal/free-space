//! `status`: the one-line sentence (T13, shared with bare `freespace`, D10)
//! followed by the details, or the full `StatusResponse` under `--json`.

use clap::Args;
use freespace_core::Result;
use freespace_core::config;
use freespace_core::format;
use freespace_core::queries::{self, diff, types::StatusResponse};
use freespace_core::snapshot::SnapshotDir;
use freespace_core::snapshot::lock::Scanning;
use freespace_core::snapshot::progress::Progress;

use crate::output;
use crate::scan::count;

#[derive(Args)]
pub struct StatusArgs {
    #[arg(long)]
    pub json: bool,
}

pub struct Summary {
    pub status: StatusResponse,
    /// "+9 G since prev in /home/tass/Downloads": the deepest folder carrying
    /// at least half of the growth of the first root.
    pub growth: Option<(String, i64)>,
}

pub fn gather(dir: &SnapshotDir) -> Result<Summary> {
    let status = queries::status(dir)?;
    let growth = match (&status.prev, status.roots.first()) {
        (Some(_), Some(root)) => diff::open_pair(dir).ok().and_then(|(cur, prev)| {
            diff::top_growth(&cur, &prev, root.path.as_bytes())
                .ok()
                .flatten()
        }),
        _ => None,
    };
    Ok(Summary { status, growth })
}

/// "14 G free · snapshot 7 h old · +9.0 G since prev in /home/tass/Downloads · scanning / since 12:03"
pub fn sentence(s: &Summary) -> String {
    let r = &s.status;
    let mut parts = Vec::new();
    if r.generation.is_none()
        && let Some(sc) = &r.scanning
    {
        return scanning_line(sc, r.progress.as_ref(), true);
    }
    if let Some(fs) = r.fs.first() {
        let avail = fs.avail.map(format::size).unwrap_or_else(|| "?".into());
        if r.fs.len() > 1 {
            parts.push(format!("{avail} free on {}", output::escape(&fs.path)));
        } else {
            parts.push(format!("{avail} free"));
        }
    }
    if let Some(age) = r.age_seconds {
        parts.push(format!("snapshot {} old", format::age(age)));
    }
    if let Some((path, d)) = &s.growth {
        parts.push(format!(
            "{} since prev in {}",
            format::delta(*d),
            output::escape(path)
        ));
    }
    if let Some(sc) = &r.scanning {
        parts.push(scanning_line(sc, None, false));
    }
    if r.incomplete {
        parts.push(format!("{} unreadable folders", r.unknown_subtrees.len()));
    }
    parts.join(" · ")
}

/// "Scanning / since 12:03 · 2.1 M of ~7.3 M files · 29% · 48 s" (D12/D14).
pub fn scanning_line(sc: &Scanning, p: Option<&Progress>, capital: bool) -> String {
    let verb = match (sc.op.as_str(), capital) {
        ("refresh", true) => "Refreshing",
        ("refresh", false) => "refreshing",
        (_, true) => "Scanning",
        (_, false) => "scanning",
    };
    let mut s = format!(
        "{verb} {} since {}",
        output::escape(&sc.path),
        format::local_hm(sc.started_at)
    );
    if let Some(p) = p {
        let expected = p
            .entries_expected
            .map(|e| format!(" of ~{}", count(e)))
            .unwrap_or_default();
        s.push_str(&format!(" · {}{expected} files", count(p.entries_seen)));
        if let Some(pc) = p.percent {
            s.push_str(&format!(" · {pc}%"));
        }
        s.push_str(&format!(" · {} s", p.elapsed_seconds));
    }
    s
}

/// D12: "Run: freespace doctor" when the snapshot is stale or the last run failed.
pub fn doctor_hint(r: &StatusResponse) -> Option<String> {
    let stale = r
        .age_seconds
        .is_some_and(|a| a > crate::doctor::STALE_SECONDS);
    let failed = r.last_error.is_some();
    (stale || failed).then(|| "Run: freespace doctor".to_owned())
}

pub fn run(a: &StatusArgs) -> output::Result<()> {
    let dir = config::load()?.snapshot_dir();
    let s = gather(&dir)?;
    if a.json {
        output::print_json(&s.status)?;
        return Ok(());
    }
    output::print(&details(&s))?;
    Ok(())
}

fn stamp(ts: i64) -> String {
    format!("{} {}", format::local_date(ts), format::local_hm(ts))
}

fn append_snapshot_rows(r: &StatusResponse, rows: &mut Vec<(String, String)>) {
    rows.push((
        "host".into(),
        format!("{} · {} · freespace {}", r.host, r.os, r.version),
    ));
    match (r.generation, r.scanned_at) {
        (Some(g), Some(at)) => rows.push((
            "snapshot".into(),
            format!(
                "generation {g} · {} entries · scanned {} · {}",
                count(r.entries.unwrap_or_default()),
                stamp(at),
                r.snapshot_dir
            ),
        )),
        _ => rows.push(("snapshot".into(), format!("none yet in {}", r.snapshot_dir))),
    }
    if let Some(p) = &r.prev {
        rows.push((
            "previous".into(),
            format!(
                "generation {} · scanned {}",
                p.generation,
                stamp(p.scanned_at)
            ),
        ));
    }
    for (root, fs) in r.roots.iter().zip(&r.fs) {
        let free = fs
            .avail
            .map(|a| format!("{} free now", format::size(a)))
            .unwrap_or_else(|| "not mounted now".into());
        rows.push((
            "root".into(),
            format!(
                "{} · {} · {} in {} items · filesystem {} used of {}, {free}",
                root.path,
                root.fs_type,
                format::size(root.allocated),
                count(root.items),
                format::size(fs.used),
                format::size(fs.total)
            ),
        ));
    }
}

fn details(s: &Summary) -> String {
    let r = &s.status;
    let mut rows: Vec<(String, String)> = Vec::new();
    append_snapshot_rows(r, &mut rows);
    for x in &r.refreshed {
        rows.push((
            "refreshed".into(),
            format!(
                "{} at {} (generation {})",
                x.path,
                stamp(x.scanned_at),
                x.generation
            ),
        ));
    }
    if let Some(sc) = &r.scanning {
        rows.push((
            "running".into(),
            format!(
                "{} · pid {}",
                scanning_line(sc, r.progress.as_ref(), false),
                sc.pid
            ),
        ));
    }
    if let Some(e) = &r.last_error {
        rows.push((
            "last error".into(),
            format!("{} {}: {}", stamp(e.ts), e.op, e.message),
        ));
    }
    if let Some(ts) = r.last_success_ts {
        rows.push(("last success".into(), stamp(ts)));
    }
    for w in &r.warnings {
        rows.push(("warning".into(), w.clone()));
    }
    if let Some(e) = &r.state_error {
        rows.push(("state.json".into(), format!("unreadable: {e}")));
    }
    if r.incomplete {
        let n = r.unknown_subtrees.len();
        let list: Vec<&str> = r
            .unknown_subtrees
            .iter()
            .take(5)
            .map(String::as_str)
            .collect();
        let more = if n > 5 {
            format!(" and {} more", n - 5)
        } else {
            String::new()
        };
        rows.push((
            "incomplete".into(),
            format!("unreadable, sizes unknown: {}{more}", list.join(", ")),
        ));
    }
    let w = rows
        .iter()
        .map(|(k, _)| k.chars().count())
        .max()
        .unwrap_or(0);
    let mut out = sentence(s);
    out.push('\n');
    for (k, v) in rows {
        out.push_str(&format!("{k:<w$}  {}\n", output::escape(&v)));
    }
    if let Some(h) = doctor_hint(r) {
        out.push_str(&h);
        out.push('\n');
    }
    out
}

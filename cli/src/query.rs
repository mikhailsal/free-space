//! Look commands: `ls`, `top`, `find`, `diff`. Each maps its flags onto the
//! core params struct (the same one `/api/*` fills from the query string) and
//! prints either the JSON answer or a plain table.

use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;

use clap::Args;
use freespace_core::format;
use freespace_core::queries::params::{DiffParams, FindParams, LsParams, TopParams, hex_encode};
use freespace_core::queries::types::{DiffEntry, DiffStatus};
use freespace_core::scan::Kind;
use freespace_core::{Error, Result};

use crate::output::{self, Align, Table};
use crate::scan::count;

mod display;
use display::{
    diff_path_cell, empty_note, items, items_delta, more_line, name_cell, path_cell, stamp,
    status_badge,
};

#[derive(Args)]
pub struct PathOpts {
    /// Folder in the snapshot (default /); relative paths start at the current directory
    #[arg(value_hint = clap::ValueHint::DirPath)]
    pub path: Option<PathBuf>,
    /// The path as hex bytes, for names that are not UTF-8 (`path_raw` in JSON answers)
    #[arg(long, value_name = "HEX", conflicts_with = "path")]
    pub path_raw: Option<String>,
}

#[derive(Args)]
pub struct LsArgs {
    #[command(flatten)]
    pub at: PathOpts,
    /// allocated | apparent | mtime (oldest first)
    #[arg(long, value_name = "KEY", value_parser = ["allocated", "apparent", "mtime"])]
    pub sort: Option<String>,
    /// Rows to show (default 30, max 10000)
    #[arg(long)]
    pub limit: Option<u32>,
    /// Fail with ROTATED unless the current snapshot has this snapshot_id
    #[arg(long, value_name = "ID")]
    pub snapshot: Option<String>,
    #[arg(long)]
    pub json: bool,
}

#[derive(Args)]
pub struct TopArgs {
    #[command(flatten)]
    pub at: PathOpts,
    /// file (default) | dir: topmost folders, none counted twice
    #[arg(long, value_parser = ["file", "dir"])]
    pub kind: Option<String>,
    /// Only entries at least this big: 500M, 1G, 2T
    #[arg(long, value_name = "SIZE")]
    pub min_size: Option<String>,
    /// Files modified (folders: nothing inside modified) before: 180d, 6mo, 1y, 2024-01-31
    #[arg(long, value_name = "AGE")]
    pub older_than: Option<String>,
    /// allocated | apparent | mtime (oldest first)
    #[arg(long, value_name = "KEY", value_parser = ["allocated", "apparent", "mtime"])]
    pub sort: Option<String>,
    #[arg(long)]
    pub limit: Option<u32>,
    #[arg(long)]
    pub json: bool,
}

#[derive(Args)]
pub struct FindArgs {
    #[command(flatten)]
    pub at: PathOpts,
    /// Name glob, case-sensitive: '*.iso', 'node_modules'
    #[arg(long, value_name = "GLOB")]
    pub name: Option<String>,
    /// Name glob, ignoring case
    #[arg(long, value_name = "GLOB", conflicts_with = "name")]
    pub iname: Option<String>,
    /// Modified before: 180d, 4w, 6mo, 1y, 2024-01-31 (implies --kind file unless given)
    #[arg(long, value_name = "AGE")]
    pub older_than: Option<String>,
    /// Modified after: same forms as --older-than
    #[arg(long, value_name = "AGE")]
    pub newer_than: Option<String>,
    #[arg(long, value_name = "SIZE")]
    pub min_size: Option<String>,
    /// file | dir | any (default any)
    #[arg(long, value_parser = ["file", "dir", "any"])]
    pub kind: Option<String>,
    /// allocated | apparent | mtime (oldest first)
    #[arg(long, value_name = "KEY", value_parser = ["allocated", "apparent", "mtime"])]
    pub sort: Option<String>,
    #[arg(long)]
    pub limit: Option<u32>,
    #[arg(long, value_name = "ID")]
    pub snapshot: Option<String>,
    #[arg(long)]
    pub json: bool,
}

#[derive(Args)]
pub struct DiffArgs {
    #[command(flatten)]
    pub at: PathOpts,
    /// Levels below the path to compare (default 1)
    #[arg(long)]
    pub depth: Option<u32>,
    /// dir (default) | file | any
    #[arg(long, value_parser = ["file", "dir", "any"])]
    pub kind: Option<String>,
    #[arg(long)]
    pub limit: Option<u32>,
    #[arg(long)]
    pub json: bool,
}

/// `(path, path_raw)` for the params structs: relative paths are made absolute
/// against the current directory (symlinks resolved when the path still exists),
/// non-UTF-8 paths travel as hex.
fn path_params(o: &PathOpts) -> Result<(Option<String>, Option<String>)> {
    let Some(p) = &o.path else {
        return Ok((None, o.path_raw.clone()));
    };
    let abs = if p.is_absolute() {
        p.clone()
    } else {
        let cwd = std::env::current_dir()
            .map_err(|e| Error::usage(format!("cannot resolve {}: {e}", p.display())))?;
        let joined = cwd.join(p);
        std::fs::canonicalize(&joined).unwrap_or(joined)
    };
    Ok(match abs.to_str() {
        Some(s) => (Some(s.to_owned()), None),
        None => (None, Some(hex_encode(abs.as_os_str().as_bytes()))),
    })
}

pub fn ls(a: &LsArgs) -> output::Result<()> {
    let (path, path_raw) = path_params(&a.at)?;
    let snap = crate::backend::Session::open(&crate::backend::load()?)?;
    let r = snap.ls(&LsParams {
        path,
        path_raw,
        sort: a.sort.clone(),
        limit: a.limit,
        snapshot: a.snapshot.clone(),
    })?;
    output::warn_incomplete(&r.header);
    if a.json {
        output::print_json(&r)?;
        return Ok(());
    }
    let e = &r.entry;
    let mut out = format!(
        "{}  {} · {} items · snapshot {} old\n",
        output::escape(&e.path),
        format::size(e.allocated),
        count(e.items),
        format::age(r.header.age_seconds)
    );
    if r.entries.is_empty() {
        out.push_str(&empty_note(e));
        output::print(&out)?;
        return Ok(());
    }
    let mut t = Table::new(&[
        ("ALLOCATED", Align::Right),
        ("SHARE", Align::Right),
        ("ITEMS", Align::Right),
        ("MODIFIED", Align::Left),
        ("NAME", Align::Left),
    ]);
    for c in &r.entries {
        let share = (c.allocated * 100)
            .checked_div(e.allocated)
            .map_or_else(|| "-".into(), |p| format!("{p} %"));
        t.row(vec![
            format::size(c.allocated),
            share,
            items(c),
            format::local_date(c.mtime),
            name_cell(&c.name, c),
        ]);
    }
    out.push_str(&t.render());
    if r.truncated {
        out.push_str(&more_line(r.total_count, r.entries.len(), "--limit"));
    }
    output::print(&out)?;
    Ok(())
}

pub fn top(a: &TopArgs) -> output::Result<()> {
    let (path, path_raw) = path_params(&a.at)?;
    let snap = crate::backend::Session::open(&crate::backend::load()?)?;
    let r = snap.top(&TopParams {
        path,
        path_raw,
        kind: a.kind.clone(),
        min_size: a.min_size.clone(),
        older_than: a.older_than.clone(),
        sort: a.sort.clone(),
        limit: a.limit,
    })?;
    output::warn_incomplete(&r.header);
    if a.json {
        output::print_json(&r)?;
        return Ok(());
    }
    let dirs = r.kind == "dir";
    let noun = if dirs { "folders" } else { "files" };
    let mut filters = String::new();
    if let Some(m) = r.min_size {
        filters.push_str(&format!(" ≥ {}", format::size(m)));
    }
    if let Some(o) = r.older_than {
        filters.push_str(&format!(" untouched since {}", format::local_date(o)));
    }
    let mut out = format!(
        "{} {noun}{filters} under {} · {} · snapshot {} old\n",
        count(r.total.count),
        output::escape(&r.path),
        format::size(r.total.allocated),
        format::age(r.header.age_seconds)
    );
    if r.entries.is_empty() {
        out.push_str("Nothing matches.\n");
        output::print(&out)?;
        return Ok(());
    }
    let mut t = if dirs {
        Table::new(&[
            ("ALLOCATED", Align::Right),
            ("ITEMS", Align::Right),
            ("NEWEST FILE", Align::Left),
            ("PATH", Align::Left),
        ])
    } else {
        Table::new(&[
            ("ALLOCATED", Align::Right),
            ("MODIFIED", Align::Left),
            ("PATH", Align::Left),
        ])
    };
    for e in &r.entries {
        if dirs {
            t.row(vec![
                format::size(e.allocated),
                items(e),
                format::local_date(e.newest_mtime),
                path_cell(e),
            ]);
        } else {
            t.row(vec![
                format::size(e.allocated),
                format::local_date(e.mtime),
                path_cell(e),
            ]);
        }
    }
    out.push_str(&t.render());
    if r.truncated {
        out.push_str(&more_line(r.total_count, r.entries.len(), "--limit"));
    }
    output::print(&out)?;
    Ok(())
}

pub fn find(a: &FindArgs) -> output::Result<()> {
    let (path, path_raw) = path_params(&a.at)?;
    let snap = crate::backend::Session::open(&crate::backend::load()?)?;
    let r = snap.find(&FindParams {
        path,
        path_raw,
        name: a.name.clone(),
        iname: a.iname.clone(),
        older_than: a.older_than.clone(),
        newer_than: a.newer_than.clone(),
        min_size: a.min_size.clone(),
        kind: a.kind.clone(),
        sort: a.sort.clone(),
        limit: a.limit,
        snapshot: a.snapshot.clone(),
    })?;
    output::warn_incomplete(&r.header);
    if a.json {
        output::print_json(&r)?;
        return Ok(());
    }
    let nested = r.total.count - r.total.counted;
    let nested_note = if nested > 0 {
        format!(" ({} inside other matches, counted once)", count(nested))
    } else {
        String::new()
    };
    let mut out = format!(
        "{} matches under {} · {}{nested_note} · snapshot {} old\n",
        count(r.total.count),
        output::escape(&r.path),
        format::size(r.total.allocated),
        format::age(r.header.age_seconds)
    );
    if r.entries.is_empty() {
        out.push_str(&format!("No matches within {}.\n", output::escape(&r.path)));
        output::print(&out)?;
        return Ok(());
    }
    let mut t = Table::new(&[
        ("ALLOCATED", Align::Right),
        ("MODIFIED", Align::Left),
        ("PATH", Align::Left),
    ]);
    for f in &r.entries {
        let e = &f.entry;
        let when = if e.kind == Kind::Dir {
            e.newest_mtime
        } else {
            e.mtime
        };
        let mut p = path_cell(e);
        if f.nested {
            p.push_str("  (in parent total)");
        }
        t.row(vec![format::size(e.allocated), format::local_date(when), p]);
    }
    out.push_str(&t.render());
    if r.truncated {
        out.push_str(&more_line(r.total_count, r.entries.len(), "--limit"));
    }
    output::print(&out)?;
    Ok(())
}

pub fn diff(a: &DiffArgs) -> output::Result<()> {
    let (path, path_raw) = path_params(&a.at)?;
    let r = crate::backend::diff(
        &crate::backend::load()?,
        &DiffParams {
            path,
            path_raw,
            depth: a.depth,
            kind: a.kind.clone(),
            limit: a.limit,
        },
    )?;
    output::warn_incomplete(&r.header);
    if a.json {
        output::print_json(&r)?;
        return Ok(());
    }
    let what = match r.kind.as_str() {
        "dir" => "folders",
        "file" => "files",
        _ => "files and folders",
    };
    let mut out = format!(
        "{}  {} since {} (generation {}) · depth {} · {what}\n",
        output::escape(&r.path),
        format::delta(r.delta),
        stamp(r.prev_scanned_at),
        r.header.prev_generation.unwrap_or_default(),
        r.depth
    );
    if r.partial {
        let list: Vec<String> = r
            .refreshed
            .iter()
            .map(|x| output::escape(&x.path))
            .collect();
        out.push_str(&format!(
            "partial: {} rescanned after the previous scan, deltas there mix two baselines\n",
            list.join(", ")
        ));
    }
    if r.roots_changed {
        out.push_str("roots changed: the two scans covered different folders\n");
    }
    // `same` rows are part of the JSON contract; the table shows changes only.
    let changed: Vec<&DiffEntry> = r
        .entries
        .iter()
        .filter(|e| e.status != DiffStatus::Same)
        .collect();
    let same = r.entries.len() - changed.len();
    if changed.is_empty() {
        out.push_str("No changes.\n");
    } else {
        let mut t = Table::new(&[
            ("DELTA", Align::Right),
            ("ALLOCATED", Align::Right),
            ("ITEMS", Align::Right),
            ("", Align::Left),
            ("PATH", Align::Left),
        ]);
        for e in changed {
            t.row(vec![
                format::delta(e.delta),
                format::size(e.allocated),
                items_delta(e),
                status_badge(e),
                diff_path_cell(e),
            ]);
        }
        out.push_str(&t.render());
    }
    if same > 0 {
        out.push_str(&format!("{same} unchanged not shown\n"));
    }
    if r.truncated {
        out.push_str(&more_line(r.total_count, r.entries.len(), "--limit"));
    }
    output::print(&out)?;
    Ok(())
}

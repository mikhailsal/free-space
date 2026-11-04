//! Research-only coarse snapshot; not an application backend.
#[cfg(test)]
mod bucket_tests;
mod buckets;
mod model;
mod mounts;
mod state;
mod store;
#[cfg(test)]
mod tests;
mod walk;

use clap::{Parser, Subcommand};
use model::{Node, R, Stats, hex, now, threshold};
use serde_json::{Map, Value, json};
use state::{Plan, Slice};
use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::{fs, time::Instant};

#[cfg(target_env = "musl")]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[derive(Parser)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    Scan {
        #[arg(long)]
        root: PathBuf,
        #[arg(long)]
        snapshot: PathBuf,
        #[arg(long, default_value_t = 1000)]
        ppm: u32,
    },
    Refresh {
        #[arg(long)]
        root: PathBuf,
        #[arg(long)]
        snapshot: PathBuf,
        #[arg(long)]
        target: PathBuf,
    },
    Inspect {
        #[arg(long)]
        snapshot: PathBuf,
        #[arg(long)]
        path: Option<PathBuf>,
    },
}
fn capacity(root: &Path) -> R<u64> {
    let path = CString::new(root.as_os_str().as_bytes())?;
    let mut stat = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    if unsafe { libc::statvfs(path.as_ptr(), stat.as_mut_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    let stat = unsafe { stat.assume_init() };
    let size = (stat.f_blocks as u128) * (stat.f_frsize as u128);
    Ok(u64::try_from(size)?)
}
fn prepare(root: &Path, snapshot: &Path) -> R<(PathBuf, PathBuf)> {
    let root = fs::canonicalize(root)?;
    fs::create_dir_all(snapshot)?;
    let snapshot = fs::canonicalize(snapshot)?;
    if snapshot.starts_with(&root) {
        return Err("snapshot directory must be outside scan root".into());
    }
    Ok((root, snapshot))
}
fn locked(dir: &Path) -> R<fs::File> {
    use std::os::fd::AsRawFd;
    let file = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(dir.join(".coarse.lock"))?;
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err("coarse snapshot is locked".into());
    }
    Ok(file)
}
fn live_ancestor(target: &Path) -> R<PathBuf> {
    let mut candidate = target;
    loop {
        match fs::canonicalize(candidate) {
            Ok(path) => return Ok(path),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                candidate = candidate.parent().ok_or("missing target ancestor")?
            }
            Err(e) => return Err(e.into()),
        }
    }
}
fn put_stats(fields: &mut Map<String, Value>, prefix: &str, s: &Stats) {
    let mut put = |name: &str, value: Value| {
        fields.insert(format!("{prefix}{name}"), value);
    };
    put("allocated", s.allocated.into());
    put("apparent", s.apparent.into());
    put("files", s.files.into());
    put("dirs", s.dirs.into());
    put("others", s.others.into());
    put("errors", s.errors.into());
    put("links", s.links.into());
    put("min_scanned_at", s.min_scanned_at.into());
    put("newest_mtime", s.newest_mtime.into());
}
fn inspect_node(n: &Node, detail: bool) -> R<Value> {
    let mut fields = Map::new();
    fields.insert("path_hex".into(), hex(&n.path).into());
    fields.insert("parent_hex".into(), n.parent.as_deref().map(hex).into());
    fields.insert("kind".into(), n.kind.into());
    fields.insert("own_allocated".into(), n.own_allocated.into());
    fields.insert("own_apparent".into(), n.own_apparent.into());
    fields.insert("own_errors".into(), n.own_errors.into());
    fields.insert("scanned_at".into(), n.scanned_at.into());
    fields.insert("bucket_count".into(), n.buckets.len().into());
    put_stats(&mut fields, "", &n.total);
    put_stats(&mut fields, "rest_", &n.rest()?);
    if detail {
        let list: Vec<Value> = n
            .buckets
            .iter()
            .map(|b| json!({"lo":b.lo,"hi":b.hi,"entries":b.stats.entries(),"allocated":b.stats.allocated,"links":b.stats.links}))
            .collect();
        fields.insert("buckets".into(), Value::Array(list));
    }
    Ok(Value::Object(fields))
}
fn inspect(snapshot: &Path, path: Option<PathBuf>) -> R<Value> {
    let (m, nodes) = store::read(&snapshot.join("coarse.db"))?;
    let selected = match path {
        Some(p) => Some(state::nearest(&nodes, &fs::canonicalize(p)?)?),
        None => None,
    };
    let rows = nodes
        .iter()
        .filter(|n| {
            selected
                .as_ref()
                .is_none_or(|p| &n.path == p || n.parent.as_ref() == Some(p))
        })
        .map(|n| inspect_node(n, selected.as_ref() == Some(&n.path)))
        .collect::<R<Vec<_>>>()?;
    Ok(
        json!({"success":true,"operation":"inspect","meta":{"root_hex":hex(&m.root),"threshold":m.threshold,"ppm":m.ppm,"capacity":m.capacity,"generation":m.generation,"scanned_at":m.scanned_at,"has_hardlinks":m.has_hardlinks},"nodes":rows}),
    )
}
fn initial_state(
    operation: &str,
    db: &Path,
    root: &Path,
    ppm: Option<u32>,
) -> R<(store::Meta, Vec<Node>)> {
    if operation == "refresh" {
        let (m, n) = store::read(db)?;
        if m.root != root {
            return Err("snapshot root mismatch".into());
        }
        return Ok((m, n));
    }
    let cap = capacity(root)?;
    let ppm = ppm.ok_or("missing ppm")?;
    let generation = if db.exists() {
        store::read(db)?
            .0
            .generation
            .checked_add(1)
            .ok_or("generation overflow")?
    } else {
        1
    };
    let meta = store::Meta {
        root: root.to_path_buf(),
        threshold: threshold(cap, ppm)?,
        ppm,
        capacity: cap,
        generation,
        scanned_at: now(),
        has_hardlinks: false,
    };
    Ok((meta, Vec::new()))
}

/// What a scan or refresh actually did, for the report.
struct Outcome {
    boundary: PathBuf,
    scope: &'static str,
    fallback: bool,
    visited: u64,
    /// Slot range, entries it held before and entries it holds now, for range refreshes.
    range: Option<(u32, u32, u64, u64)>,
}
fn rescan_root(
    root: &Path,
    meta: &store::Meta,
    nodes: &mut Vec<Node>,
    spent: u64,
    fallback: bool,
) -> R<Outcome> {
    let result = walk::scan(root, meta.threshold)?;
    let visited = spent.checked_add(result.visited).ok_or("visit overflow")?;
    *nodes = result.nodes;
    Ok(Outcome {
        boundary: root.to_path_buf(),
        scope: "full",
        fallback,
        visited,
        range: None,
    })
}
fn refresh_retained(
    root: &Path,
    meta: &store::Meta,
    nodes: &mut Vec<Node>,
    boundary: PathBuf,
) -> R<Outcome> {
    let result = walk::scan(&boundary, meta.threshold)?;
    if result.has_hardlinks {
        return rescan_root(root, meta, nodes, result.visited, true);
    }
    let visited = result.visited;
    state::update(nodes, &boundary, result.nodes, meta.threshold)?;
    Ok(Outcome {
        boundary,
        scope: "retained",
        fallback: false,
        visited,
        range: None,
    })
}
fn refresh_range(
    root: &Path,
    meta: &store::Meta,
    nodes: &mut Vec<Node>,
    slice: Slice,
) -> R<Outcome> {
    let Slice {
        parent,
        lo,
        hi,
        old,
    } = slice;
    let scope = state::scope(nodes, &parent, lo, hi);
    let result = walk::scan_range(&parent, meta.threshold, &scope)?;
    if result.has_hardlinks {
        return rescan_root(root, meta, nodes, result.visited, true);
    }
    let fresh: u64 = result.buckets.iter().map(|b| b.stats.entries()).sum();
    let visited = result.visited;
    state::update_range(nodes, &parent, &scope, result, meta.threshold)?;
    Ok(Outcome {
        boundary: parent,
        scope: "range",
        fallback: false,
        visited,
        range: Some((lo, hi, old.entries(), fresh)),
    })
}
fn refresh(root: &Path, meta: &store::Meta, nodes: &mut Vec<Node>, target: &Path) -> R<Outcome> {
    let target = live_ancestor(target)?;
    if !target.starts_with(root) {
        return Err("refresh target outside root".into());
    }
    let plan = state::plan(nodes, &target)?;
    if matches!(&plan, Plan::Retained(b) if b == root) {
        return rescan_root(root, meta, nodes, 0, target != root);
    }
    if plan.links(nodes) > 0 {
        return rescan_root(root, meta, nodes, 0, true);
    }
    match plan {
        Plan::Retained(boundary) => refresh_retained(root, meta, nodes, boundary),
        Plan::Range(slice) => refresh_range(root, meta, nodes, slice),
    }
}
fn persist(snapshot: &Path, meta: &store::Meta, nodes: &[Node]) -> R<()> {
    let tmp = snapshot.join(".coarse.tmp");
    if tmp.exists() {
        return Err("stale temporary snapshot".into());
    }
    let saved = store::write(&tmp, meta, nodes).and_then(|()| store::publish(snapshot, &tmp));
    if saved.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    saved
}
fn report(
    operation: &str,
    root: &Path,
    meta: &store::Meta,
    nodes: &[Node],
    o: &Outcome,
    start: Instant,
) -> Value {
    let buckets: usize = nodes.iter().map(|n| n.buckets.len()).sum();
    let range = o
        .range
        .map(|(lo, hi, old, new)| json!({"lo":lo,"hi":hi,"old_entries":old,"new_entries":new}));
    json!({"success":true,"operation":operation,"root":root.to_string_lossy(),"threshold":meta.threshold,"capacity":meta.capacity,
        "generation":meta.generation,"visited":o.visited,"retained":nodes.len(),"buckets":buckets,"boundary":o.boundary.to_string_lossy(),
        "scope":o.scope,"fallback_full_root":o.fallback,"range":range,"elapsed_seconds":start.elapsed().as_secs_f64()})
}
fn execute(command: Command) -> R<Value> {
    let start = Instant::now();
    let (operation, root, snapshot, target, ppm) = match command {
        Command::Inspect { snapshot, path } => return inspect(&snapshot, path),
        Command::Scan {
            root,
            snapshot,
            ppm,
        } => ("scan", root, snapshot, None, Some(ppm)),
        Command::Refresh {
            root,
            snapshot,
            target,
        } => ("refresh", root, snapshot, Some(target), None),
    };
    let (root, snapshot) = prepare(&root, &snapshot)?;
    let _lock = locked(&snapshot)?;
    let (mut meta, mut nodes) = initial_state(operation, &snapshot.join("coarse.db"), &root, ppm)?;
    let outcome = match target {
        Some(target) => {
            meta.generation = meta
                .generation
                .checked_add(1)
                .ok_or("generation overflow")?;
            refresh(&root, &meta, &mut nodes, &target)?
        }
        None => rescan_root(&root, &meta, &mut nodes, 0, false)?,
    };
    meta.has_hardlinks = state::links(&nodes) > 0;
    meta.scanned_at = now();
    persist(&snapshot, &meta, &nodes)?;
    Ok(report(operation, &root, &meta, &nodes, &outcome, start))
}
fn main() {
    match Cli::try_parse()
        .map_err(|e| e.to_string())
        .and_then(|cli| execute(cli.command).map_err(|e| e.to_string()))
    {
        Ok(v) => println!("{v}"),
        Err(e) => {
            println!("{}", json!({"success":false,"error":e}));
            std::process::exit(1);
        }
    }
}

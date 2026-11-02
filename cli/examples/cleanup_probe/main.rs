//! Research-only coarse snapshot; not an application backend.
mod model;
mod mounts;
mod store;
#[cfg(test)]
mod tests;
mod walk;

use clap::{Parser, Subcommand};
use model::{MAX_RECORDS, Node, R, hex, now, threshold};
use serde_json::{Value, json};
use std::collections::HashMap;
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
fn rebuild(nodes: &mut [Node]) -> R<()> {
    let mut indices: Vec<usize> = (0..nodes.len()).collect();
    indices.sort_by_key(|i| std::cmp::Reverse(nodes[*i].path.components().count()));
    let lookup: HashMap<PathBuf, usize> = nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.path.clone(), i))
        .collect();
    for n in nodes.iter_mut() {
        let mut t = n.own();
        t.add(&n.rest)?;
        n.total = t;
    }
    for i in indices {
        if let Some(parent) = nodes[i].parent.clone() {
            let j = *lookup.get(&parent).ok_or("retained node missing parent")?;
            let child = nodes[i].total.clone();
            nodes[j].total.add(&child)?;
        }
    }
    Ok(())
}
fn nearest(nodes: &[Node], target: &Path) -> R<PathBuf> {
    nodes
        .iter()
        .filter(|n| n.kind == 1 && target.starts_with(&n.path))
        .max_by_key(|n| n.path.components().count())
        .map(|n| n.path.clone())
        .ok_or_else(|| "no retained ancestor".into())
}
fn update(
    old: &mut Vec<Node>,
    boundary: &Path,
    mut replacement: Vec<Node>,
    threshold: u64,
) -> R<()> {
    let prior = old
        .iter()
        .find(|n| n.path == boundary)
        .ok_or("boundary missing")?
        .clone();
    let root = replacement
        .iter_mut()
        .find(|n| n.path == boundary)
        .ok_or("replacement root missing")?;
    root.parent = prior.parent.clone();
    let fold = root.total.allocated < threshold && prior.parent.is_some();
    let new_total = root.total.clone();
    old.retain(|n| !n.path.starts_with(boundary));
    if fold {
        if replacement.len() != 1 {
            return Err("small subtree contains retained descendant".into());
        }
        let parent = old
            .iter_mut()
            .find(|n| Some(&n.path) == prior.parent.as_ref())
            .ok_or("parent missing")?;
        parent.rest.add(&new_total)?;
    } else {
        old.extend(replacement);
    }
    if old.len() > MAX_RECORDS {
        return Err("retained record cap exceeded".into());
    }
    rebuild(old)?;
    prune(old, threshold)
}
fn prune(nodes: &mut Vec<Node>, threshold: u64) -> R<()> {
    let mut candidates: Vec<usize> = nodes
        .iter()
        .enumerate()
        .filter(|(_, n)| n.parent.is_some() && n.total.allocated < threshold)
        .map(|(i, _)| i)
        .collect();
    candidates.sort_by_key(|i| std::cmp::Reverse(nodes[*i].path.components().count()));
    let lookup: HashMap<PathBuf, usize> = nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.path.clone(), i))
        .collect();
    let mut children: HashMap<PathBuf, usize> = HashMap::new();
    for node in nodes.iter() {
        if let Some(parent) = &node.parent {
            *children.entry(parent.clone()).or_default() += 1;
        }
    }
    let mut removed = std::collections::HashSet::new();
    for i in candidates {
        if children.get(&nodes[i].path).copied().unwrap_or(0) != 0 {
            return Err("small ancestor contains retained child".into());
        }
        let parent = nodes[i].parent.as_ref().ok_or("parent missing")?.clone();
        let j = *lookup.get(&parent).ok_or("parent missing")?;
        let total = nodes[i].total.clone();
        nodes[j].rest.add(&total)?;
        *children.get_mut(&parent).ok_or("child count missing")? -= 1;
        removed.insert(nodes[i].path.clone());
    }
    nodes.retain(|n| !removed.contains(&n.path));
    rebuild(nodes)
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
fn inspect_node(n: &Node) -> Value {
    json!({"path_hex":hex(&n.path),"parent_hex":n.parent.as_ref().map(|p|hex(p)),"kind":n.kind,
        "own_allocated":n.own_allocated,"own_apparent":n.own_apparent,"own_errors":n.own_errors,
        "allocated":n.total.allocated,"apparent":n.total.apparent,"files":n.total.files,"dirs":n.total.dirs,"others":n.total.others,"errors":n.total.errors,
        "scanned_at":n.scanned_at,"min_scanned_at":n.total.min_scanned_at,"newest_mtime":n.total.newest_mtime,
        "rest_allocated":n.rest.allocated,"rest_apparent":n.rest.apparent,"rest_files":n.rest.files,"rest_dirs":n.rest.dirs,
        "rest_others":n.rest.others,"rest_errors":n.rest.errors,"rest_min_scanned_at":n.rest.min_scanned_at,"rest_newest_mtime":n.rest.newest_mtime})
}
fn inspect(snapshot: &Path, path: Option<PathBuf>) -> R<Value> {
    let (m, nodes) = store::read(&snapshot.join("coarse.db"))?;
    let selected = if let Some(p) = path {
        let p = fs::canonicalize(p)?;
        Some(nearest(&nodes, &p)?)
    } else {
        None
    };
    let rows: Vec<_> = nodes
        .iter()
        .filter(|n| {
            selected
                .as_ref()
                .is_none_or(|p| &n.path == p || n.parent.as_ref() == Some(p))
        })
        .map(inspect_node)
        .collect();
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
        Ok((m, n))
    } else {
        let cap = capacity(root)?;
        let ppm = ppm.ok_or("missing ppm")?;
        let t = threshold(cap, ppm)?;
        let generation = if db.exists() {
            store::read(db)?
                .0
                .generation
                .checked_add(1)
                .ok_or("generation overflow")?
        } else {
            1
        };
        Ok((
            store::Meta {
                root: root.to_path_buf(),
                threshold: t,
                ppm,
                capacity: cap,
                generation,
                scanned_at: now(),
                has_hardlinks: false,
            },
            Vec::new(),
        ))
    }
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
    let db = snapshot.join("coarse.db");
    let (mut meta, mut nodes) = initial_state(operation, &db, &root, ppm)?;
    let mut boundary = root.clone();
    let mut fallback = false;
    if let Some(target) = target {
        let target = live_ancestor(&target)?;
        if !target.starts_with(&root) {
            return Err("refresh target outside root".into());
        }
        boundary = nearest(&nodes, &target)?;
        if meta.has_hardlinks {
            boundary = root.clone();
        }
        fallback = target != root && boundary == root;
    }
    let mut result = walk::scan(&boundary, meta.threshold)?;
    let mut visited = result.visited;
    if operation == "refresh" && boundary != root && result.has_hardlinks {
        fallback = true;
        boundary = root.clone();
        result = walk::scan(&boundary, meta.threshold)?;
        visited = visited
            .checked_add(result.visited)
            .ok_or("visit overflow")?;
    }
    if operation == "refresh" {
        meta.generation = meta
            .generation
            .checked_add(1)
            .ok_or("generation overflow")?;
    }
    if boundary == root {
        nodes = result.nodes;
        meta.has_hardlinks = result.has_hardlinks;
    } else {
        update(&mut nodes, &boundary, result.nodes, meta.threshold)?;
    }
    meta.scanned_at = now();
    let tmp = snapshot.join(".coarse.tmp");
    if tmp.exists() {
        return Err("stale temporary snapshot".into());
    }
    let saved = store::write(&tmp, &meta, &nodes).and_then(|()| store::publish(&snapshot, &tmp));
    if saved.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    saved?;
    Ok(
        json!({"success":true,"operation":operation,"root":root.to_string_lossy(),"threshold":meta.threshold,"capacity":meta.capacity,
        "generation":meta.generation,"visited":visited,"retained":nodes.len(),"boundary":boundary.to_string_lossy(),
        "fallback_full_root":fallback,"elapsed_seconds":start.elapsed().as_secs_f64()}),
    )
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

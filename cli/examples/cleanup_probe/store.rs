use crate::model::{
    BUCKET_ENTRIES, Bucket, FORMAT, MAX_BUCKETS, MAX_RECORDS, Node, R, SLOT_BITS, Stats,
};
use rusqlite::{Connection, params, params_from_iter, types::Value};
use std::collections::HashMap;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::{ffi::OsString, fs};

pub struct Meta {
    pub root: PathBuf,
    pub threshold: u64,
    pub ppm: u32,
    pub capacity: u64,
    pub generation: u64,
    pub scanned_at: i64,
    pub has_hardlinks: bool,
}
const CREATE: &str = "CREATE TABLE meta(singleton INTEGER PRIMARY KEY,root BLOB NOT NULL,threshold INTEGER NOT NULL,ppm INTEGER NOT NULL,capacity INTEGER NOT NULL,generation INTEGER NOT NULL,scanned_at INTEGER NOT NULL,has_hardlinks INTEGER NOT NULL,format INTEGER NOT NULL,slot_bits INTEGER NOT NULL,bucket_entries INTEGER NOT NULL);\
CREATE TABLE nodes(path BLOB PRIMARY KEY,parent BLOB,kind INTEGER NOT NULL,own_allocated INTEGER NOT NULL,own_apparent INTEGER NOT NULL,own_errors INTEGER NOT NULL,scanned_at INTEGER NOT NULL,allocated INTEGER NOT NULL,apparent INTEGER NOT NULL,files INTEGER NOT NULL,dirs INTEGER NOT NULL,others INTEGER NOT NULL,errors INTEGER NOT NULL,links INTEGER NOT NULL,min_scanned_at INTEGER,newest_mtime INTEGER) WITHOUT ROWID; CREATE INDEX nodes_parent ON nodes(parent);\
CREATE TABLE buckets(parent BLOB NOT NULL,lo INTEGER NOT NULL,hi INTEGER NOT NULL,allocated INTEGER NOT NULL,apparent INTEGER NOT NULL,files INTEGER NOT NULL,dirs INTEGER NOT NULL,others INTEGER NOT NULL,errors INTEGER NOT NULL,links INTEGER NOT NULL,min_scanned_at INTEGER,newest_mtime INTEGER,PRIMARY KEY(parent,lo)) WITHOUT ROWID;";
const NODE_COLUMNS: &str = "path,parent,kind,own_allocated,own_apparent,own_errors,scanned_at,allocated,apparent,files,dirs,others,errors,links,min_scanned_at,newest_mtime";
const BUCKET_COLUMNS: &str =
    "parent,lo,hi,allocated,apparent,files,dirs,others,errors,links,min_scanned_at,newest_mtime";

fn num(x: u64) -> R<Value> {
    Ok(Value::Integer(i64::try_from(x)?))
}
fn opt(x: Option<i64>) -> Value {
    x.map_or(Value::Null, Value::Integer)
}
fn blob(path: &Path) -> Value {
    Value::Blob(path.as_os_str().as_bytes().to_vec())
}
fn stat_values(s: &Stats) -> R<Vec<Value>> {
    Ok(vec![
        num(s.allocated)?,
        num(s.apparent)?,
        num(s.files)?,
        num(s.dirs)?,
        num(s.others)?,
        num(s.errors)?,
        num(s.links)?,
        opt(s.min_scanned_at),
        opt(s.newest_mtime),
    ])
}
fn node_values(n: &Node) -> R<Vec<Value>> {
    let mut v = vec![
        blob(&n.path),
        n.parent.as_deref().map_or(Value::Null, blob),
        Value::Integer(n.kind.into()),
        num(n.own_allocated)?,
        num(n.own_apparent)?,
        num(n.own_errors)?,
        Value::Integer(n.scanned_at),
    ];
    v.extend(stat_values(&n.total)?);
    Ok(v)
}
fn bucket_values(parent: &Path, b: &Bucket) -> R<Vec<Value>> {
    let mut v = vec![
        blob(parent),
        Value::Integer(b.lo.into()),
        Value::Integer(b.hi.into()),
    ];
    v.extend(stat_values(&b.stats)?);
    Ok(v)
}
fn placeholders(count: usize) -> String {
    (1..=count)
        .map(|i| format!("?{i}"))
        .collect::<Vec<_>>()
        .join(",")
}
pub fn write(path: &Path, m: &Meta, nodes: &[Node]) -> R<()> {
    let buckets: usize = nodes.iter().map(|n| n.buckets.len()).sum();
    if nodes.len() > MAX_RECORDS || buckets > MAX_BUCKETS {
        return Err("snapshot exceeds record cap".into());
    }
    let mut c = Connection::open(path)?;
    c.execute_batch("PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL;")?;
    c.execute_batch(CREATE)?;
    let tx = c.transaction()?;
    tx.execute(
        "INSERT INTO meta VALUES(1,?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
        params![
            m.root.as_os_str().as_bytes(),
            i64::try_from(m.threshold)?,
            m.ppm,
            i64::try_from(m.capacity)?,
            i64::try_from(m.generation)?,
            m.scanned_at,
            m.has_hardlinks,
            FORMAT,
            SLOT_BITS,
            i64::try_from(BUCKET_ENTRIES)?
        ],
    )?;
    {
        let mut q = tx.prepare(&format!(
            "INSERT INTO nodes({NODE_COLUMNS}) VALUES({})",
            placeholders(16)
        ))?;
        let mut b = tx.prepare(&format!(
            "INSERT INTO buckets({BUCKET_COLUMNS}) VALUES({})",
            placeholders(12)
        ))?;
        for n in nodes {
            q.execute(params_from_iter(node_values(n)?))?;
            for bucket in &n.buckets {
                b.execute(params_from_iter(bucket_values(&n.path, bucket)?))?;
            }
        }
    }
    tx.commit()?;
    c.close().map_err(|(_, e)| e)?;
    fs::File::open(path)?.sync_all()?;
    Ok(())
}
fn decode(v: Vec<u8>) -> PathBuf {
    PathBuf::from(OsString::from_vec(v))
}
fn unsigned(r: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<u64> {
    let value: i64 = r.get(index)?;
    u64::try_from(value).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(index, value))
}
fn stats_at(r: &rusqlite::Row<'_>, at: usize) -> rusqlite::Result<Stats> {
    Ok(Stats {
        allocated: unsigned(r, at)?,
        apparent: unsigned(r, at + 1)?,
        files: unsigned(r, at + 2)?,
        dirs: unsigned(r, at + 3)?,
        others: unsigned(r, at + 4)?,
        errors: unsigned(r, at + 5)?,
        links: unsigned(r, at + 6)?,
        min_scanned_at: r.get(at + 7)?,
        newest_mtime: r.get(at + 8)?,
    })
}
fn node_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Node> {
    Ok(Node {
        path: decode(r.get(0)?),
        parent: r.get::<_, Option<Vec<u8>>>(1)?.map(decode),
        kind: r.get(2)?,
        own_allocated: unsigned(r, 3)?,
        own_apparent: unsigned(r, 4)?,
        own_errors: unsigned(r, 5)?,
        scanned_at: r.get(6)?,
        total: stats_at(r, 7)?,
        buckets: Vec::new(),
    })
}
fn bucket_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<(PathBuf, Bucket)> {
    let bucket = Bucket {
        lo: r.get(1)?,
        hi: r.get(2)?,
        stats: stats_at(r, 3)?,
    };
    Ok((decode(r.get(0)?), bucket))
}
fn read_meta(c: &Connection) -> R<Meta> {
    let count: i64 = c.query_row("SELECT count(*) FROM meta", [], |r| r.get(0))?;
    if count != 1 {
        return Err("snapshot must have exactly one meta row".into());
    }
    let (meta, format, slot_bits, bucket_entries) = c.query_row(
        "SELECT root,threshold,ppm,capacity,generation,scanned_at,has_hardlinks,format,slot_bits,bucket_entries FROM meta WHERE singleton=1",
        [],
        |r| {
            let meta = Meta {
                root: decode(r.get(0)?),
                threshold: unsigned(r, 1)?,
                ppm: r.get(2)?,
                capacity: unsigned(r, 3)?,
                generation: unsigned(r, 4)?,
                scanned_at: r.get(5)?,
                has_hardlinks: r.get(6)?,
            };
            Ok((meta, r.get::<_, i64>(7)?, r.get::<_, u32>(8)?, unsigned(r, 9)?))
        },
    )?;
    if format != FORMAT || slot_bits != SLOT_BITS || bucket_entries != BUCKET_ENTRIES {
        return Err("unsupported snapshot format".into());
    }
    Ok(meta)
}
/// Hand each directory its buckets; leftovers mean the snapshot is inconsistent.
fn attach(nodes: &mut [Node], rows: Vec<(PathBuf, Bucket)>) -> R<()> {
    let mut by_parent: HashMap<PathBuf, Vec<Bucket>> = HashMap::new();
    for (parent, bucket) in rows {
        by_parent.entry(parent).or_default().push(bucket);
    }
    for n in nodes.iter_mut().filter(|n| n.kind == 1) {
        n.buckets = by_parent.remove(&n.path).unwrap_or_default();
    }
    if !by_parent.is_empty() {
        return Err("buckets reference a missing or non-directory node".into());
    }
    Ok(())
}
pub fn read(path: &Path) -> R<(Meta, Vec<Node>)> {
    let c = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let meta = read_meta(&c)?;
    let mut q = c.prepare(&format!(
        "SELECT {NODE_COLUMNS} FROM nodes ORDER BY path LIMIT ?1"
    ))?;
    let mut nodes = q
        .query_map([MAX_RECORDS as i64 + 1], node_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if nodes.len() > MAX_RECORDS {
        return Err("snapshot exceeds record cap".into());
    }
    let mut q = c.prepare(&format!(
        "SELECT {BUCKET_COLUMNS} FROM buckets ORDER BY parent,lo LIMIT ?1"
    ))?;
    let rows = q
        .query_map([MAX_BUCKETS as i64 + 1], bucket_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if rows.len() > MAX_BUCKETS {
        return Err("snapshot exceeds bucket cap".into());
    }
    attach(&mut nodes, rows)?;
    crate::state::verify(&nodes)?;
    Ok((meta, nodes))
}
pub fn publish(dir: &Path, temp: &Path) -> R<()> {
    let current = dir.join("coarse.db");
    let previous = dir.join("coarse.prev.db");
    // Copy the old generation first; replacing the current path is then one atomic rename.
    if current.exists() {
        let prev_temp = dir.join(".coarse-prev.tmp");
        if prev_temp.exists() {
            return Err("stale previous temporary file".into());
        }
        fs::copy(&current, &prev_temp)?;
        fs::File::open(&prev_temp)?.sync_all()?;
        fs::rename(&prev_temp, &previous)?;
        fs::File::open(dir)?.sync_all()?;
    }
    fs::rename(temp, &current)?;
    fs::File::open(dir)?.sync_all()?;
    Ok(())
}

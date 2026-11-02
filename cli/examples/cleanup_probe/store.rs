use crate::model::{Node, R, Stats};
use rusqlite::{Connection, params, params_from_iter, types::Value};
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
const CREATE: &str = "CREATE TABLE meta(singleton INTEGER PRIMARY KEY,root BLOB NOT NULL,threshold INTEGER NOT NULL,ppm INTEGER NOT NULL,capacity INTEGER NOT NULL,generation INTEGER NOT NULL,scanned_at INTEGER NOT NULL,has_hardlinks INTEGER NOT NULL);\
CREATE TABLE nodes(path BLOB PRIMARY KEY,parent BLOB,kind INTEGER NOT NULL,own_allocated INTEGER NOT NULL,own_apparent INTEGER NOT NULL,own_errors INTEGER NOT NULL,allocated INTEGER NOT NULL,apparent INTEGER NOT NULL,files INTEGER NOT NULL,dirs INTEGER NOT NULL,others INTEGER NOT NULL,errors INTEGER NOT NULL,scanned_at INTEGER NOT NULL,min_scanned_at INTEGER,newest_mtime INTEGER,rest_allocated INTEGER NOT NULL,rest_apparent INTEGER NOT NULL,rest_files INTEGER NOT NULL,rest_dirs INTEGER NOT NULL,rest_others INTEGER NOT NULL,rest_errors INTEGER NOT NULL,rest_min_scanned_at INTEGER,rest_newest_mtime INTEGER) WITHOUT ROWID; CREATE INDEX nodes_parent ON nodes(parent);";
fn num(x: u64) -> R<Value> {
    Ok(Value::Integer(i64::try_from(x)?))
}
fn opt(x: Option<i64>) -> Value {
    x.map_or(Value::Null, Value::Integer)
}
fn values(n: &Node) -> R<Vec<Value>> {
    let t = &n.total;
    let r = &n.rest;
    Ok(vec![
        Value::Blob(n.path.as_os_str().as_bytes().to_vec()),
        n.parent.as_ref().map_or(Value::Null, |p| {
            Value::Blob(p.as_os_str().as_bytes().to_vec())
        }),
        Value::Integer(n.kind.into()),
        num(n.own_allocated)?,
        num(n.own_apparent)?,
        num(n.own_errors)?,
        num(t.allocated)?,
        num(t.apparent)?,
        num(t.files)?,
        num(t.dirs)?,
        num(t.others)?,
        num(t.errors)?,
        Value::Integer(n.scanned_at),
        opt(t.min_scanned_at),
        opt(t.newest_mtime),
        num(r.allocated)?,
        num(r.apparent)?,
        num(r.files)?,
        num(r.dirs)?,
        num(r.others)?,
        num(r.errors)?,
        opt(r.min_scanned_at),
        opt(r.newest_mtime),
    ])
}
pub fn write(path: &Path, m: &Meta, nodes: &[Node]) -> R<()> {
    let mut c = Connection::open(path)?;
    c.execute_batch("PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL;")?;
    c.execute_batch(CREATE)?;
    let tx = c.transaction()?;
    tx.execute(
        "INSERT INTO meta VALUES(1,?1,?2,?3,?4,?5,?6,?7)",
        params![
            m.root.as_os_str().as_bytes(),
            i64::try_from(m.threshold)?,
            m.ppm,
            i64::try_from(m.capacity)?,
            i64::try_from(m.generation)?,
            m.scanned_at,
            m.has_hardlinks
        ],
    )?;
    {
        let mut q=tx.prepare("INSERT INTO nodes VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23)")?;
        for n in nodes {
            q.execute(params_from_iter(values(n)?))?;
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
fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Node> {
    let u = |i| r.get::<_, u64>(i);
    Ok(Node {
        path: decode(r.get(0)?),
        parent: r.get::<_, Option<Vec<u8>>>(1)?.map(decode),
        kind: r.get(2)?,
        own_allocated: u(3)?,
        own_apparent: u(4)?,
        own_errors: u(5)?,
        total: Stats {
            allocated: u(6)?,
            apparent: u(7)?,
            files: u(8)?,
            dirs: u(9)?,
            others: u(10)?,
            errors: u(11)?,
            min_scanned_at: r.get(13)?,
            newest_mtime: r.get(14)?,
        },
        scanned_at: r.get(12)?,
        rest: Stats {
            allocated: u(15)?,
            apparent: u(16)?,
            files: u(17)?,
            dirs: u(18)?,
            others: u(19)?,
            errors: u(20)?,
            min_scanned_at: r.get(21)?,
            newest_mtime: r.get(22)?,
        },
    })
}
pub fn read(path: &Path) -> R<(Meta, Vec<Node>)> {
    let c = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let count: i64 = c.query_row("SELECT count(*) FROM meta", [], |r| r.get(0))?;
    if count != 1 {
        return Err("snapshot must have exactly one meta row".into());
    }
    let m=c.query_row("SELECT root,threshold,ppm,capacity,generation,scanned_at,has_hardlinks FROM meta WHERE singleton=1",[],|r|Ok(Meta{root:decode(r.get(0)?),threshold:r.get(1)?,ppm:r.get(2)?,capacity:r.get(3)?,generation:r.get(4)?,scanned_at:r.get(5)?,has_hardlinks:r.get(6)?}))?;
    let mut q = c.prepare("SELECT * FROM nodes ORDER BY path LIMIT ?1")?;
    let nodes = q
        .query_map([crate::model::MAX_RECORDS as i64 + 1], row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if nodes.len() > crate::model::MAX_RECORDS {
        return Err("snapshot exceeds record cap".into());
    }
    Ok((m, nodes))
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

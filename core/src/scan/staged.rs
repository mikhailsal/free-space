//! Opt-in disk-backed full scan. No per-entry collection survives in RAM.
//! SQLite B-tree indexes are maintained from empty tables, avoiding sort runs.

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::Instant;

use rusqlite::{Connection, OptionalExtension, params};

use super::denied::DeniedPaths;
use super::tree::{Kind, Node, SubtreeAgg, Tree, flags};
use super::walk::{RootCtx, WalkOptions, prepare_roots};
use crate::error::{Error, Result};
use crate::platform;
use crate::snapshot::fsutil;
use crate::snapshot::writer::{self, Reserve, WriteMeta, WriteStats};

mod walk;
mod write;

/// Cache target per connection. Two connections are open during final writing.
const CACHE_KIB: u64 = 8192;

pub struct StagedTree {
    conn: Connection,
    path: PathBuf,
    roots: Tree, // root metadata only, never the discovered entries
    count: u64,
    one_file_system: bool,
    reserve: Reserve,
}

impl Drop for StagedTree {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Checks available space independently of output-file growth: staging and the
/// denied-path spool can grow while the final snapshot size remains unchanged.
struct SpaceGuard {
    reserve: Reserve,
    ticks: u64,
}
impl SpaceGuard {
    fn new(reserve: &Reserve) -> Result<Self> {
        let guard = Self {
            reserve: reserve.clone(),
            ticks: 0,
        };
        guard.check()?;
        Ok(guard)
    }
    fn check(&self) -> Result<()> {
        let st = platform::statvfs(&self.reserve.dir)
            .map_err(|e| Error::io("check scan temporary space", e))?;
        if st.avail < self.reserve.min_free {
            return Err(writer::reserve_error(
                st.avail,
                self.reserve.min_free,
                &self.reserve.dir,
            ));
        }
        Ok(())
    }
    fn tick(&mut self) -> Result<()> {
        self.ticks += 1;
        if self.ticks.is_multiple_of(1024) {
            self.check()?;
        }
        Ok(())
    }
}

fn open_database(path: &Path) -> Result<Connection> {
    use std::os::unix::fs::OpenOptionsExt;
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o640)
        .open(path)
        .map_err(|e| Error::io(format!("create {}", path.display()), e))?;
    fsutil::secure_file(&file).map_err(|e| Error::io("secure scan database", e))?;
    let conn = Connection::open(path)?;
    conn.execute_batch(&format!(
        "PRAGMA journal_mode=OFF; PRAGMA synchronous=OFF; PRAGMA temp_store=FILE;
         PRAGMA cache_size=-{CACHE_KIB}; PRAGMA mmap_size=0; PRAGMA cache_spill=ON;
         PRAGMA threads=0; PRAGMA locking_mode=EXCLUSIVE;"
    ))?;
    Ok(conn)
}

struct WalkContext<'a> {
    opts: &'a WalkOptions,
    roots: &'a [RootCtx],
    mounts: &'a [platform::MountInfo],
    stage_path: &'a Path,
    guard: &'a mut SpaceGuard,
}

impl StagedTree {
    pub fn walk(opts: &WalkOptions, path: &Path, reserve: &Reserve) -> Result<Self> {
        let result = Self::walk_inner(opts, path, reserve);
        if result.is_err() {
            let _ = std::fs::remove_file(path);
        }
        result
    }

    fn walk_inner(opts: &WalkOptions, path: &Path, reserve: &Reserve) -> Result<Self> {
        let mut guard = SpaceGuard::new(reserve)?;
        let mounts = platform::visible_mountinfo();
        let mut roots = Tree {
            started_at: platform::now_secs(),
            ..Default::default()
        };
        let mut root_opts = opts.clone();
        root_opts.expected = None; // used-inode estimates must never allocate a full tree
        let (paths, contexts) = prepare_roots(&root_opts, &mounts, roots.started_at, &mut roots)
            .map_err(super::walk_error)?;
        let conn = open_database(path)?;
        conn.execute_batch("BEGIN;
            CREATE TABLE raw(id INTEGER PRIMARY KEY, parent INTEGER, name BLOB NOT NULL,
              kind INTEGER NOT NULL, flags INTEGER NOT NULL, allocated INTEGER NOT NULL,
              apparent INTEGER NOT NULL, mtime INTEGER NOT NULL, nlink INTEGER NOT NULL,
              errors INTEGER NOT NULL, scanned_at INTEGER NOT NULL);
            CREATE INDEX raw_children ON raw(parent,name,id);
            CREATE TABLE jobs(id INTEGER PRIMARY KEY, root INTEGER NOT NULL, path BLOB NOT NULL);
            CREATE TABLE links(id INTEGER PRIMARY KEY, dev INTEGER NOT NULL, ino INTEGER NOT NULL,
              path BLOB NOT NULL);
            CREATE INDEX links_owner ON links(dev,ino,path);
            CREATE TABLE mount_info(id INTEGER PRIMARY KEY, path BLOB NOT NULL, fs_type TEXT NOT NULL,
              fs_used INTEGER, fs_total INTEGER);")?;
        let count = roots.nodes.len() as u64;
        let mut tree = Self {
            conn,
            path: path.to_owned(),
            roots,
            count,
            one_file_system: opts.one_file_system,
            reserve: reserve.clone(),
        };
        for (id, path) in paths.iter().enumerate() {
            let node = tree.roots.nodes[id];
            tree.insert(id as u64, None, tree.roots.name(&node), &node)?;
            tree.job(id as u64, id as u64, path)?;
        }
        opts.seen.fetch_add(count, Ordering::Relaxed);
        let stage_path =
            std::fs::canonicalize(path).map_err(|e| Error::io("resolve staging path", e))?;
        let mut context = WalkContext {
            opts,
            roots: &contexts,
            mounts: &mounts,
            stage_path: &stage_path,
            guard: &mut guard,
        };
        while let Some(job) = tree.next_job()? {
            tree.read_directory(job, &mut context)?;
        }
        tree.conn.execute_batch("COMMIT")?;
        guard.check()?;
        Ok(tree)
    }

    pub fn len(&self) -> u64 {
        self.count
    }
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }
    pub fn started_at(&self) -> i64 {
        self.roots.started_at
    }

    fn insert(&self, id: u64, parent: Option<u64>, name: &[u8], n: &Node) -> Result<()> {
        self.conn
            .prepare_cached("INSERT INTO raw VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)")?
            .execute(params![
                id as i64,
                parent.map(|v| v as i64),
                name,
                n.kind,
                n.flags,
                n.allocated as i64,
                n.apparent as i64,
                n.mtime,
                n.nlink,
                n.errors,
                n.scanned_at
            ])?;
        Ok(())
    }
}

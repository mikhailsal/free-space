use super::*;

mod batch;
use std::os::unix::ffi::OsStrExt;

use crate::scan::tree::{path_hash, push_component};
use crate::snapshot::schema::{CREATE_INDEXES, CREATE_TABLES};

struct Raw {
    id: u64,
    name: Vec<u8>,
    node: Node,
}
struct Frame {
    raw: u64,
    id: u64,
    kind: Kind,
    cursor: Option<(Vec<u8>, u64)>,
    path_len: usize,
    agg: SubtreeAgg,
    pending: batch::Pending,
}

impl Raw {
    fn from_sql(r: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: r.get::<_, i64>(0)? as u64,
            name: r.get(1)?,
            node: Node {
                kind: r.get(2)?,
                flags: r.get(3)?,
                allocated: r.get::<_, i64>(4)? as u64,
                apparent: r.get::<_, i64>(5)? as u64,
                mtime: r.get(6)?,
                nlink: r.get(7)?,
                errors: r.get(8)?,
                scanned_at: r.get(9)?,
                ..Default::default()
            },
        })
    }
}
const RAW_COLS: &str = "id,name,kind,flags,allocated,apparent,mtime,nlink,errors,scanned_at";

impl StagedTree {
    /// Finalization and row insertion happen together; no permutation or rank
    /// vector is allocated. The caller publishes only after this closes/fsyncs.
    pub fn write(self, path: &Path, meta: &WriteMeta) -> Result<(WriteStats, DeniedPaths, u64)> {
        let result = self.write_inner(path, meta);
        if result.is_err() {
            let _ = std::fs::remove_file(path);
        }
        result
    }

    fn write_inner(&self, path: &Path, meta: &WriteMeta) -> Result<(WriteStats, DeniedPaths, u64)> {
        let mut guard = SpaceGuard::new(&self.reserve)?;
        let conn = open_database(path)?;
        let mut denied = DeniedPaths::on_disk(&self.reserve.dir)?;
        let begin = Instant::now();
        conn.execute_batch("BEGIN")?;
        conn.execute_batch(CREATE_TABLES)?;
        // Maintain B-trees incrementally. CREATE INDEX after bulk inserts would
        // need sort buffers and temp files outside our explicit staging file.
        for sql in CREATE_INDEXES {
            conn.execute_batch(sql)?;
        }
        writer::insert_metadata(&conn, self.count, meta, &Error::from)?;
        let mut next_id = 0;
        let mut errors = 0;
        for root in 0..self.roots.roots.len() {
            let id = next_id;
            errors += self.write_root(&conn, root as u64, &mut next_id, &mut denied, &mut guard)?;
            let info = &self.roots.roots[root];
            conn.execute(
                "INSERT INTO roots VALUES (?1,?2,?3,?4,?5,?6,?7)",
                params![
                    id as i64,
                    info.path.as_os_str().as_bytes(),
                    info.fs_type,
                    info.stats.used as i64,
                    info.stats.total as i64,
                    info.stats.avail as i64,
                    info.stats.inodes_used().map(|v| v as i64)
                ],
            )?;
        }
        if next_id != self.count {
            return Err(Error::generic(
                "staging traversal did not visit every entry",
            ));
        }
        conn.execute_batch("COMMIT")?;
        let insert_seconds = begin.elapsed().as_secs_f64();
        crate::scan::profile::phase("index");
        let index = Instant::now();
        conn.execute_batch("PRAGMA analysis_limit=1000; ANALYZE;")?;
        guard.check()?;
        conn.close().map_err(|(_, e)| Error::from(e))?;
        let stats = WriteStats {
            rows: self.count,
            bytes: writer::sync_snapshot(path)?,
            insert_seconds,
            index_seconds: index.elapsed().as_secs_f64(),
        };
        Ok((stats, denied, errors))
    }

    fn write_root(
        &self,
        conn: &Connection,
        root: u64,
        next_id: &mut u64,
        denied: &mut DeniedPaths,
        guard: &mut SpaceGuard,
    ) -> Result<u64> {
        let raw = self.conn.query_row(
            &format!("SELECT {RAW_COLS} FROM raw WHERE id=?1"),
            [root as i64],
            Raw::from_sql,
        )?;
        let mut path = raw.name.clone();
        let first = self.enter(conn, raw, None, *next_id, &path, denied)?;
        *next_id += 1;
        let mut stack = vec![first];
        let mut budget = batch::Budget::default();
        loop {
            guard.tick()?;
            evict_for_directory(&mut stack, &mut budget);
            let parent = stack.last_mut().unwrap();
            if let Some(raw) = batch::next(
                &self.conn,
                parent.raw,
                &parent.cursor,
                &mut parent.pending,
                &mut budget,
            )? {
                parent.cursor = Some((raw.name.clone(), raw.id));
                path.truncate(parent.path_len);
                push_component(&mut path, &raw.name);
                let frame = self.enter(conn, raw, Some(parent.id), *next_id, &path, denied)?;
                *next_id += 1;
                stack.push(frame);
                continue;
            }
            let frame = stack.pop().unwrap();
            conn.prepare_cached(
                "UPDATE entries SET allocated=?2,apparent=?3,items=?4,errors=?5,
                    min_scanned_at=?6,newest_mtime=?7,subtree_end=?8 WHERE id=?1",
            )?
            .execute(params![
                frame.id as i64,
                frame.agg.allocated as i64,
                frame.agg.apparent as i64,
                frame.agg.items as i64,
                frame.agg.errors as i64,
                frame.agg.min_scanned_at,
                frame.agg.stored_newest_mtime(),
                (frame.id + frame.agg.items) as i64
            ])?;
            if let Some(parent) = stack.last_mut() {
                parent.agg.merge(&frame.agg);
                // Match Tree's per-merge u32 saturation.
                let mut node = Node::default();
                parent.agg.store(&mut node);
                parent.agg = SubtreeAgg::of(&node);
            } else {
                return Ok(frame.agg.errors);
            }
        }
    }

    fn enter(
        &self,
        conn: &Connection,
        raw: Raw,
        parent: Option<u64>,
        id: u64,
        path: &[u8],
        denied: &mut DeniedPaths,
    ) -> Result<Frame> {
        let mut node = raw.node;
        self.attribute_link(conn, raw.id, id, &mut node)?;
        let agg = SubtreeAgg::leaf(
            node.kind(),
            node.allocated,
            node.apparent,
            node.mtime,
            node.scanned_at,
            node.errors as u64,
        );
        conn.prepare_cached(
            "INSERT INTO entries VALUES
            (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,0,?11,?12,?1,?12,?13)",
        )?
        .execute(params![
            id as i64,
            parent.map(|v| v as i64),
            raw.name,
            path_hash(path),
            node.kind,
            node.flags,
            node.allocated as i64,
            node.apparent as i64,
            node.mtime,
            node.nlink,
            node.errors,
            node.scanned_at,
            agg.stored_newest_mtime()
        ])?;
        if node.flags & flags::DENIED != 0 {
            conn.execute("INSERT INTO denied VALUES (?1)", [id as i64])?;
            denied.push(path)?;
        }
        if node.kind() == Kind::Mount {
            let (path, fs_type, used, total): (Vec<u8>, String, Option<i64>, Option<i64>) =
                self.conn.query_row(
                    "SELECT path,fs_type,fs_used,fs_total FROM mount_info WHERE id=?1",
                    [raw.id as i64],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                )?;
            conn.execute(
                "INSERT INTO mounts VALUES (?1,?2,?3,?4,?5)",
                params![id as i64, path, fs_type, used, total],
            )?;
        }
        Ok(Frame {
            raw: raw.id,
            id,
            kind: node.kind(),
            cursor: None,
            path_len: path.len(),
            agg,
            pending: batch::Pending::default(),
        })
    }

    fn attribute_link(&self, conn: &Connection, raw: u64, id: u64, node: &mut Node) -> Result<()> {
        if node.kind() == Kind::Dir || node.nlink <= 1 {
            return Ok(());
        }
        // Identical full paths can appear in overlapping roots across filesystems.
        // Tree keeps the first DFS occurrence, so ties follow original root order.
        let link: Option<(i64, i64, i64)> = self
            .conn
            .prepare_cached(
                "SELECT l.dev,l.ino,
            (SELECT o.id FROM links o WHERE o.dev=l.dev AND o.ino=l.ino ORDER BY o.path,o.root,o.id LIMIT 1)
            FROM links l WHERE l.id=?1",
            )?
            .query_row([raw as i64], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .optional()?;
        if let Some((dev, ino, owner)) = link {
            conn.prepare_cached("INSERT INTO hardlinks VALUES (?1,?2,?3,?4,?5)")?
                .execute(params![
                    id as i64,
                    dev,
                    ino,
                    node.allocated as i64,
                    node.apparent as i64
                ])?;
            if owner as u64 != raw {
                node.flags |= flags::ALIAS;
                node.allocated = 0;
                node.apparent = 0;
            }
        }
        Ok(())
    }
}

// Evicted ancestors resume from their last consumed cursor when revisited.
fn evict_for_directory(stack: &mut [Frame], budget: &mut batch::Budget) {
    let Some((current, ancestors)) = stack.split_last_mut() else {
        return;
    };
    if current.kind != Kind::Dir || !current.pending.is_empty() {
        return;
    }
    for ancestor in ancestors {
        if budget.can_buffer() {
            break;
        }
        if !ancestor.pending.is_empty() {
            ancestor.pending.release(budget);
        }
    }
}

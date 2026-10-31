//! Refresh-only on-disk staging and replacement subtree ingestion.
use super::*;
use crate::snapshot::reader::Snapshot;
use std::os::unix::ffi::OsStrExt;

mod policy;

impl StagedTree {
    pub(crate) fn refresh(
        snap: &Snapshot,
        roots: Vec<super::super::RootInfo>,
        target: i64,
        end: i64,
        sub: Option<Tree>,
        path: &Path,
        reserve: &Reserve,
    ) -> Result<Self> {
        if sub.is_none() {
            let is_root: bool = snap.conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM roots WHERE entry_id=?1)",
                [target],
                |row| row.get(0),
            )?;
            if is_root {
                return Err(Error::usage("a missing root requires a full scan"));
            }
        }
        let mut guard = SpaceGuard::new(reserve)?;
        let conn = open_database(path)?;
        let mut staged = Self {
            conn,
            path: path.to_owned(),
            roots: Tree {
                roots,
                started_at: snap.meta.scanned_at,
                ..Tree::default()
            },
            count: 0,
            one_file_system: snap.meta.one_file_system,
            reserve: reserve.clone(),
            cleanup: true,
            refresh_mode: true,
        };
        staged.refresh_ingest(snap, target, end, sub.as_ref(), &mut guard)?;
        guard.check()?;
        Ok(staged)
    }

    fn refresh_ingest(
        &mut self,
        snap: &Snapshot,
        target: i64,
        end: i64,
        sub: Option<&Tree>,
        guard: &mut SpaceGuard,
    ) -> Result<()> {
        self.conn.execute_batch("BEGIN")?;
        let n = super::refresh_ingest::import_nodes(&snap.conn, &self.conn, guard)?;
        super::refresh_ingest::subtract_children(&self.conn, n, guard)?;
        super::refresh_ingest::import_links(&snap.conn, &self.conn, guard)?;
        self.map_old(&snap.conn, n, guard)?;
        self.replace_old(snap, target, end, sub, n, guard)?;
        policy::prepare(self, &snap.conn, target, end, guard)?;
        self.conn.execute_batch("COMMIT")?;
        Ok(())
    }

    fn map_old(&self, source: &Connection, n: u64, guard: &mut SpaceGuard) -> Result<()> {
        self.conn.execute_batch("CREATE TABLE old_map(old_id INTEGER PRIMARY KEY, stage_id INTEGER UNIQUE NOT NULL);
            CREATE TABLE raw(id INTEGER PRIMARY KEY, parent INTEGER, name BLOB NOT NULL,
              kind INTEGER NOT NULL, flags INTEGER NOT NULL, allocated INTEGER NOT NULL,
              apparent INTEGER NOT NULL, mtime INTEGER NOT NULL, nlink INTEGER NOT NULL,
              errors INTEGER NOT NULL, scanned_at INTEGER NOT NULL);
            CREATE INDEX raw_children ON raw(parent,name,id);
            CREATE TABLE links(id INTEGER PRIMARY KEY, dev INTEGER NOT NULL, ino INTEGER NOT NULL,
              path BLOB NOT NULL, root INTEGER NOT NULL);
            CREATE INDEX links_owner ON links(dev,ino,path,root,id);
            CREATE TABLE new_link(id INTEGER NOT NULL,dev INTEGER NOT NULL,ino INTEGER NOT NULL);
            CREATE INDEX new_link_key ON new_link(dev,ino);
            CREATE TABLE mount_info(id INTEGER PRIMARY KEY, path BLOB NOT NULL, fs_type TEXT NOT NULL,
              fs_used INTEGER, fs_total INTEGER);")?;
        let mut root_stmt =
            source.prepare_cached("SELECT entry_id FROM roots ORDER BY entry_id")?;
        let mut root_rows = root_stmt.query([])?;
        let mut next = 0i64;
        while let Some(row) = root_rows.next()? {
            self.conn
                .prepare_cached("INSERT INTO old_map VALUES (?1,?2)")?
                .execute(params![row.get::<_, i64>(0)?, next])?;
            next += 1;
            guard.tick()?;
        }
        if next as usize != self.roots.roots.len() {
            return Err(Error::generic("snapshot root count changed"));
        }
        for old in 0..n {
            let inserted = self
                .conn
                .prepare_cached("INSERT OR IGNORE INTO old_map VALUES (?1,?2)")?
                .execute(params![old as i64, next])?;
            if inserted == 1 {
                next += 1;
            }
            guard.tick()?;
        }
        if next as u64 != n {
            return Err(Error::generic("snapshot root mapping is invalid"));
        }
        for old in 0..n {
            let changed = self.conn.prepare_cached("INSERT INTO raw SELECT m.stage_id,p.stage_id,r.name,r.kind,r.flags,r.allocated,r.apparent,r.mtime,r.nlink,r.errors,r.scanned_at FROM old_raw r JOIN old_map m ON m.old_id=r.old_id LEFT JOIN old_map p ON p.old_id=r.parent_id WHERE r.old_id=?1 AND (r.parent_id IS NULL OR p.stage_id IS NOT NULL)")?.execute( [old as i64])?;
            if changed != 1 {
                return Err(Error::generic("snapshot parent mapping is invalid"));
            }
            guard.tick()?;
        }
        Ok(())
    }

    fn replace_old(
        &mut self,
        snap: &Snapshot,
        target: i64,
        end: i64,
        sub: Option<&Tree>,
        n: u64,
        guard: &mut SpaceGuard,
    ) -> Result<()> {
        if target < 0 || end < target || end as u64 >= n {
            return Err(Error::generic("invalid refresh subtree range"));
        }
        let x: i64 = self
            .conn
            .prepare_cached("SELECT stage_id FROM old_map WHERE old_id=?1")?
            .query_row([target], |r| r.get(0))?;
        for old in (target + 1)..=end {
            let id: i64 = self
                .conn
                .prepare_cached("SELECT stage_id FROM old_map WHERE old_id=?1")?
                .query_row([old], |r| r.get(0))?;
            self.conn
                .prepare_cached("DELETE FROM raw WHERE id=?1")?
                .execute([id])?;
            guard.tick()?;
        }
        if sub.is_none() {
            self.conn
                .prepare_cached("DELETE FROM raw WHERE id=?1")?
                .execute([x])?;
        }
        self.copy_old_links(target, end, sub.is_some(), guard)?;
        self.copy_mounts(&snap.conn, target, end, sub.is_some(), guard)?;
        if let Some(sub) = sub {
            self.insert_subtree(x, n, sub, guard)?;
        }
        let count: i64 = self
            .conn
            .prepare_cached("SELECT count(*) FROM raw")?
            .query_row([], |r| r.get(0))?;
        self.count = u64::try_from(count).map_err(|_| Error::generic("negative staged count"))?;
        Ok(())
    }

    fn copy_old_links(
        &self,
        target: i64,
        end: i64,
        keep_root: bool,
        guard: &mut SpaceGuard,
    ) -> Result<()> {
        let mut stmt = self.conn.prepare_cached("SELECT l.old_id,m.stage_id,l.dev,l.ino FROM old_link l JOIN old_map m ON m.old_id=l.old_id ORDER BY l.seq")?;
        let mut rows = stmt.query([])?;
        while let Some(r) = rows.next()? {
            let old: i64 = r.get(0)?;
            if (old <= target || old > end) && (keep_root || old != target) {
                self.conn
                    .prepare_cached("INSERT INTO links VALUES (?1,?2,?3,X'',0)")?
                    .execute(params![
                        r.get::<_, i64>(1)?,
                        r.get::<_, i64>(2)?,
                        r.get::<_, i64>(3)?
                    ])?;
            }
            guard.tick()?;
        }
        Ok(())
    }

    fn copy_mounts(
        &self,
        source: &Connection,
        target: i64,
        end: i64,
        keep_root: bool,
        guard: &mut SpaceGuard,
    ) -> Result<()> {
        let mut stmt =
            source.prepare_cached("SELECT entry_id,path,fs_type,fs_used,fs_total FROM mounts")?;
        let mut rows = stmt.query([])?;
        while let Some(r) = rows.next()? {
            let old: i64 = r.get(0)?;
            if (old <= target || old > end) && (keep_root || old != target) {
                let id: i64 = self
                    .conn
                    .prepare_cached("SELECT stage_id FROM old_map WHERE old_id=?1")?
                    .query_row([old], |q| q.get(0))?;
                self.conn
                    .prepare_cached("INSERT INTO mount_info VALUES (?1,?2,?3,?4,?5)")?
                    .execute(params![
                        id,
                        r.get::<_, Vec<u8>>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, Option<i64>>(3)?,
                        r.get::<_, Option<i64>>(4)?
                    ])?;
            }
            guard.tick()?;
        }
        Ok(())
    }

    fn insert_subtree(&self, x: i64, n: u64, sub: &Tree, guard: &mut SpaceGuard) -> Result<()> {
        n.checked_add(sub.nodes.len().saturating_sub(1) as u64)
            .filter(|v| *v <= u32::MAX as u64)
            .ok_or_else(|| Error::generic("refresh node count exceeds raw ID capacity"))?;
        let root = sub
            .nodes
            .first()
            .ok_or_else(|| Error::generic("empty replacement subtree"))?;
        self.conn.prepare_cached("UPDATE raw SET flags=?2,allocated=?3,apparent=?4,mtime=?5,nlink=?6,errors=?7,scanned_at=?8 WHERE id=?1")?.execute(params![x,root.flags,root.allocated as i64,root.apparent as i64,root.mtime,root.nlink,root.errors,root.scanned_at])?;
        for (i, node) in sub.nodes.iter().enumerate().skip(1) {
            let id = n + i as u64 - 1;
            let parent = if node.parent == 0 {
                x
            } else {
                (n + u64::from(node.parent) - 1) as i64
            };
            self.insert(id, Some(parent as u64), sub.name(node), node)?;
            guard.tick()?;
        }
        for &(raw, dev, ino) in &sub.hardlinks {
            let id = if raw == 0 {
                x
            } else {
                (n + u64::from(raw) - 1) as i64
            };
            self.conn
                .prepare_cached("INSERT INTO links VALUES (?1,?2,?3,X'',0)")?
                .execute(params![id, dev as i64, ino as i64])?;
            self.conn
                .prepare_cached("INSERT INTO new_link VALUES (?1,?2,?3)")?
                .execute(params![id, dev as i64, ino as i64])?;
            self.conn
                .prepare_cached("INSERT OR IGNORE INTO link_keys VALUES (?1,?2)")?
                .execute(params![dev as i64, ino as i64])?;
            guard.tick()?;
        }
        for m in &sub.mounts {
            let id = if m.raw == 0 {
                x
            } else {
                (n + u64::from(m.raw) - 1) as i64
            };
            self.conn
                .prepare_cached("INSERT INTO mount_info VALUES (?1,?2,?3,?4,?5)")?
                .execute(params![
                    id,
                    m.path.as_os_str().as_bytes(),
                    m.fs_type,
                    m.stats.as_ref().map(|s| s.used as i64),
                    m.stats.as_ref().map(|s| s.total as i64)
                ])?;
        }
        Ok(())
    }
}

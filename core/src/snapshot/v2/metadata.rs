use crate::{Result, platform::FsStats, scan::RootInfo};
use rusqlite::{Connection, OptionalExtension, params};
use std::os::unix::ffi::OsStringExt;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Root {
    pub entry_id: i64,
    pub path: Vec<u8>,
    pub dev: u64,
    pub fs_type: String,
    pub stats: FsStats,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mount {
    pub entry_id: i64,
    pub path: Vec<u8>,
    pub fs_type: String,
    pub used: Option<i64>,
    pub total: Option<i64>,
}
pub(super) const ROOT_COLS: &str =
    "entry_id,path,dev,fs_type,total,free,avail,used,inodes_total,inodes_free";
pub(super) const MOUNT_COLS: &str = "entry_id,path,fs_type,used,total";
impl Root {
    pub(super) fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            entry_id: r.get(0)?,
            path: r.get(1)?,
            dev: r.get::<_, i64>(2)? as u64,
            fs_type: r.get(3)?,
            stats: FsStats {
                total: r.get::<_, i64>(4)? as u64,
                free: r.get::<_, i64>(5)? as u64,
                avail: r.get::<_, i64>(6)? as u64,
                used: r.get::<_, i64>(7)? as u64,
                inodes_total: r.get::<_, i64>(8)? as u64,
                inodes_free: r.get::<_, i64>(9)? as u64,
            },
        })
    }
    pub(super) fn info(&self) -> RootInfo {
        RootInfo {
            path: PathBuf::from(std::ffi::OsString::from_vec(self.path.clone())),
            dev: self.dev,
            fs_type: self.fs_type.clone(),
            stats: self.stats,
        }
    }
}
impl Mount {
    pub(super) fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            entry_id: r.get(0)?,
            path: r.get(1)?,
            fs_type: r.get(2)?,
            used: r.get(3)?,
            total: r.get(4)?,
        })
    }
}
pub(super) fn insert_root(c: &Connection, r: &Root, g: i64) -> Result<()> {
    c.execute(
        &format!("INSERT INTO live_roots({ROOT_COLS}) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)"),
        params![
            r.entry_id,
            r.path,
            r.dev as i64,
            r.fs_type,
            r.stats.total as i64,
            r.stats.free as i64,
            r.stats.avail as i64,
            r.stats.used as i64,
            r.stats.inodes_total as i64,
            r.stats.inodes_free as i64
        ],
    )?;
    c.execute(&format!("INSERT INTO root_versions({ROOT_COLS},born,dead) SELECT {ROOT_COLS},?1,NULL FROM live_roots WHERE entry_id=?2"),params![g,r.entry_id])?;
    Ok(())
}
pub(super) fn insert_mount(c: &Connection, m: &Mount, g: i64) -> Result<()> {
    c.execute(
        &format!("INSERT INTO live_mounts({MOUNT_COLS}) VALUES(?1,?2,?3,?4,?5)"),
        params![m.entry_id, m.path, m.fs_type, m.used, m.total],
    )?;
    c.execute(&format!("INSERT INTO mount_versions({MOUNT_COLS},born,dead) SELECT {MOUNT_COLS},?1,NULL FROM live_mounts WHERE entry_id=?2"),params![g,m.entry_id])?;
    Ok(())
}
pub(super) fn retire_root(c: &Connection, id: i64, g: i64) -> Result<()> {
    c.execute(
        "UPDATE root_versions SET dead=?1 WHERE entry_id=?2 AND dead IS NULL",
        params![g, id],
    )?;
    c.execute("DELETE FROM live_roots WHERE entry_id=?1", [id])?;
    Ok(())
}
pub(super) fn retire_mount(c: &Connection, id: i64, g: i64) -> Result<()> {
    c.execute(
        "UPDATE mount_versions SET dead=?1 WHERE entry_id=?2 AND dead IS NULL",
        params![g, id],
    )?;
    c.execute("DELETE FROM live_mounts WHERE entry_id=?1", [id])?;
    Ok(())
}
pub(super) fn live_root(c: &Connection, scope: &[u8]) -> Result<Root> {
    Ok(c.query_row(
        &format!("SELECT {ROOT_COLS} FROM live_roots WHERE path=?1"),
        [scope],
        Root::row,
    )?)
}
/// Resolve through indexed exact ancestor lookups, independent of other roots.
pub(super) fn scope(
    c: &Connection,
    path: &[u8],
    generation: Option<i64>,
) -> Result<Option<Vec<u8>>> {
    let mut ancestor = path.to_vec();
    loop {
        let found: Option<Vec<u8>> = if let Some(g) = generation {
            c.query_row("SELECT path FROM root_versions WHERE path=?1 AND born<=?2 AND (dead IS NULL OR dead>?2)",params![ancestor,g],|r|r.get(0)).optional()?
        } else {
            c.query_row(
                "SELECT path FROM live_roots WHERE path=?1",
                [&ancestor],
                |r| r.get(0),
            )
            .optional()?
        };
        if found.is_some() {
            return Ok(found);
        }
        if ancestor == b"/" {
            return Ok(None);
        }
        let pos = ancestor
            .iter()
            .rposition(|b| *b == b'/')
            .expect("normalized absolute path");
        ancestor.truncate(pos.max(1));
    }
}

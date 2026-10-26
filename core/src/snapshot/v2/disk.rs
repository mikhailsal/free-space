//! Raw staged rows become stable identities without a per-entry mapping in RAM.
use super::*;
use crate::scan::{
    flags,
    staged::visit::{Row, Visitor},
};

pub(super) struct Sink<'a> {
    pub conn: &'a Connection,
    pub generation: i64,
}
impl Visitor for Sink<'_> {
    type State = Entry;
    fn enter(&mut self, row: Row<'_>, parent: Option<&Entry>) -> Result<Entry> {
        validate_path(row.path)?;
        let scope = row.root.path.as_os_str().as_bytes();
        let id = identity(self.conn, scope, row.path)?;
        let n = row.node;
        let alias = row.link.as_ref().is_some_and(|l| !l.owner);
        let entry = Entry {
            id,
            parent: parent.map(|p| p.id),
            path: row.path.to_vec(),
            sort_key: key(row.path),
            kind: n.kind(),
            flags: if alias {
                n.flags | flags::ALIAS
            } else if row.link.is_some() {
                n.flags & !flags::ALIAS
            } else {
                n.flags
            },
            own_allocated: size(n.allocated)?,
            own_apparent: size(n.apparent)?,
            own_errors: n.errors as i64,
            mtime: n.mtime,
            scanned_at: n.scanned_at,
            allocated: if alias { 0 } else { size(n.allocated)? },
            apparent: if alias { 0 } else { size(n.apparent)? },
            items: 0,
            errors: n.errors as i64,
            min_scanned_at: n.scanned_at,
            newest_mtime: (n.kind() == Kind::File).then_some(n.mtime.min(n.scanned_at)),
            scope: scope.to_vec(),
            nlink: n.nlink as i64,
            dev: row.link.as_ref().map(|l| l.dev),
            ino: row.link.as_ref().map(|l| l.ino),
        };
        if parent.is_none() {
            metadata::insert_root(
                self.conn,
                &Root {
                    entry_id: id,
                    ordinal: row.ordinal as i64,
                    path: scope.to_vec(),
                    dev: row.root.dev,
                    fs_type: row.root.fs_type.clone(),
                    stats: row.root.stats,
                },
                self.generation,
            )?;
        }
        if let Some(mount) = row.mount {
            metadata::insert_mount(
                self.conn,
                &Mount {
                    entry_id: id,
                    path: mount.path,
                    fs_type: mount.fs_type,
                    used: mount.used,
                    total: mount.total,
                },
                self.generation,
            )?;
        }
        Ok(entry)
    }
    fn leave(&mut self, entry: Entry, parent: Option<&mut Entry>) -> Result<()> {
        if let Some(parent) = parent {
            parent.allocated = add(parent.allocated, entry.allocated)?;
            parent.apparent = add(parent.apparent, entry.apparent)?;
            parent.items = add(parent.items, add(entry.items, 1)?)?;
            parent.errors = add(parent.errors, entry.errors)?;
            parent.min_scanned_at = parent.min_scanned_at.min(entry.min_scanned_at);
            parent.newest_mtime = parent.newest_mtime.max(entry.newest_mtime);
        }
        insert(self.conn, &entry, self.generation)
    }
}

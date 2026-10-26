//! Streaming access to unaggregated scan rows. State grows with directory depth.
use super::*;
use crate::scan::{RootInfo, tree::push_component};

#[cfg(test)]
mod tests;

pub(crate) struct Link {
    pub dev: i64,
    pub ino: i64,
    pub owner: bool,
}
pub(crate) struct Mount {
    pub path: Vec<u8>,
    pub fs_type: String,
    pub used: Option<i64>,
    pub total: Option<i64>,
}
pub(crate) struct Row<'a> {
    pub node: Node,
    pub path: &'a [u8],
    pub root: &'a RootInfo,
    pub ordinal: usize,
    pub link: Option<Link>,
    pub mount: Option<Mount>,
}
pub(crate) trait Visitor {
    type State;
    fn enter(&mut self, row: Row<'_>, parent: Option<&Self::State>) -> Result<Self::State>;
    fn leave(&mut self, state: Self::State, parent: Option<&mut Self::State>) -> Result<()>;
}
struct Raw {
    id: i64,
    name: Vec<u8>,
    node: Node,
}
struct Frame<S> {
    raw: i64,
    cursor: Option<(Vec<u8>, i64)>,
    path_len: usize,
    state: S,
}
const COLUMNS: &str = "id,name,kind,flags,allocated,apparent,mtime,nlink,errors,scanned_at";
impl Raw {
    fn read(r: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: r.get(0)?,
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
impl StagedTree {
    pub(crate) fn visit<V: Visitor>(&self, visitor: &mut V) -> Result<()> {
        let mut guard = SpaceGuard::new(&self.reserve)?;
        let mut count = 0;
        for ordinal in 0..self.roots.roots.len() {
            count += self.visit_root(ordinal, visitor, &mut guard)?;
        }
        if count != self.count {
            return Err(Error::generic(
                "staging traversal did not visit every entry",
            ));
        }
        guard.check()
    }
    fn visit_root<V: Visitor>(
        &self,
        ordinal: usize,
        visitor: &mut V,
        guard: &mut SpaceGuard,
    ) -> Result<u64> {
        let raw = self.conn.query_row(
            &format!("SELECT {COLUMNS} FROM raw WHERE id=?1"),
            [ordinal as i64],
            Raw::read,
        )?;
        let mut path = raw.name.clone();
        let first = self.visit_enter(raw, &path, ordinal, visitor, None)?;
        let mut stack = vec![first];
        let mut count = 1;
        while let Some(parent) = stack.last_mut() {
            guard.tick()?;
            if let Some(raw) = self.visit_child(parent)? {
                parent.cursor = Some((raw.name.clone(), raw.id));
                path.truncate(parent.path_len);
                push_component(&mut path, &raw.name);
                let frame = self.visit_enter(raw, &path, ordinal, visitor, Some(&parent.state))?;
                stack.push(frame);
                count += 1;
            } else {
                let frame = stack.pop().unwrap();
                visitor.leave(frame.state, stack.last_mut().map(|p| &mut p.state))?;
            }
        }
        Ok(count)
    }
    fn visit_child<S>(&self, frame: &Frame<S>) -> Result<Option<Raw>> {
        Ok(if let Some((name, id)) = &frame.cursor {
            self.conn.prepare_cached(&format!("SELECT {COLUMNS} FROM raw WHERE parent=?1 AND (name,id)>(?2,?3) ORDER BY name,id LIMIT 1"))?
                .query_row(params![frame.raw, name, id], Raw::read).optional()?
        } else {
            self.conn
                .prepare_cached(&format!(
                    "SELECT {COLUMNS} FROM raw WHERE parent=?1 ORDER BY name,id LIMIT 1"
                ))?
                .query_row([frame.raw], Raw::read)
                .optional()?
        })
    }
    fn visit_enter<V: Visitor>(
        &self,
        raw: Raw,
        path: &[u8],
        ordinal: usize,
        visitor: &mut V,
        parent: Option<&V::State>,
    ) -> Result<Frame<V::State>> {
        let link = if raw.node.kind() != Kind::Dir && raw.node.nlink > 1 {
            self.conn.prepare_cached("SELECT l.dev,l.ino,l.id=(SELECT o.id FROM links o WHERE o.dev=l.dev AND o.ino=l.ino ORDER BY o.path,o.root,o.id LIMIT 1) FROM links l WHERE l.id=?1")?
                .query_row([raw.id], |r| Ok(Link {dev:r.get(0)?,ino:r.get(1)?,owner:r.get(2)?})).optional()?
        } else {
            None
        };
        let mount = if raw.node.kind() == Kind::Mount {
            Some(
                self.conn
                    .prepare_cached(
                        "SELECT path,fs_type,fs_used,fs_total FROM mount_info WHERE id=?1",
                    )?
                    .query_row([raw.id], |r| {
                        Ok(Mount {
                            path: r.get(0)?,
                            fs_type: r.get(1)?,
                            used: r.get(2)?,
                            total: r.get(3)?,
                        })
                    })?,
            )
        } else {
            None
        };
        let state = visitor.enter(
            Row {
                node: raw.node,
                path,
                root: &self.roots.roots[ordinal],
                ordinal,
                link,
                mount,
            },
            parent,
        )?;
        Ok(Frame {
            raw: raw.id,
            cursor: None,
            path_len: path.len(),
            state,
        })
    }
}

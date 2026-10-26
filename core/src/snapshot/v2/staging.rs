use super::{
    Entry, add, key,
    metadata::{Mount, Root},
    size, validate_path,
};
use crate::{
    Result,
    scan::{Kind, Tree},
};
use std::collections::HashMap;
use std::os::unix::ffi::OsStrExt;
pub(super) struct Staged {
    pub entries: Vec<Entry>,
    pub roots: Vec<Root>,
    pub mounts: Vec<Mount>,
}
pub(super) fn stage(tree: Tree, scope_override: Option<&[u8]>) -> Result<Staged> {
    let mut out: Vec<Entry> = Vec::with_capacity(tree.nodes.len());
    let mut raw_ids = HashMap::with_capacity(tree.nodes.len());
    let links: HashMap<_, _> = tree
        .hardlinks
        .iter()
        .map(|(raw, dev, ino)| (*raw, (*dev as i64, *ino as i64)))
        .collect();
    let mut stack: Vec<(u32, Option<usize>, usize)> = (0..tree.roots.len())
        .rev()
        .map(|r| (r as u32, None, r))
        .collect();
    while let Some((raw, parent, root)) = stack.pop() {
        let n = tree.nodes[raw as usize];
        let path = if let Some(p) = parent {
            let mut path = out[p].path.clone();
            crate::scan::tree::push_component(&mut path, tree.name(&n));
            path
        } else {
            tree.name(&n).to_vec()
        };
        validate_path(&path)?;
        let id = out.len();
        raw_ids.insert(raw, id as i64);
        let link = links.get(&raw).copied();
        out.push(Entry {
            id: id as i64,
            parent: parent.map(|p| p as i64),
            sort_key: key(&path),
            path,
            kind: n.kind(),
            flags: n.flags,
            own_allocated: size(n.allocated)?,
            own_apparent: size(n.apparent)?,
            own_errors: n.errors as i64,
            mtime: n.mtime,
            scanned_at: n.scanned_at,
            allocated: size(n.allocated)?,
            apparent: size(n.apparent)?,
            items: 0,
            errors: n.errors as i64,
            min_scanned_at: n.scanned_at,
            newest_mtime: (n.kind() == Kind::File).then_some(n.mtime.min(n.scanned_at)),
            scope: scope_override
                .unwrap_or_else(|| tree.roots[root].path.as_os_str().as_bytes())
                .to_vec(),
            nlink: n.nlink as i64,
            dev: link.map(|l| l.0),
            ino: link.map(|l| l.1),
        });
        for child in (n.child_start..n.child_start + n.child_count).rev() {
            stack.push((child, Some(id), root));
        }
    }
    let roots = roots(tree.roots, &raw_ids);
    let mounts = tree
        .mounts
        .into_iter()
        .map(|m| Mount {
            entry_id: raw_ids[&m.raw],
            path: m.path.as_os_str().as_bytes().to_vec(),
            fs_type: m.fs_type,
            used: m.stats.map(|s| s.used as i64),
            total: m.stats.map(|s| s.total as i64),
        })
        .collect();
    Ok(Staged {
        entries: out,
        roots,
        mounts,
    })
}
/// Run after attribution and before temporary parent IDs become stable IDs.
pub(super) fn fold(out: &mut [Entry]) -> Result<()> {
    for i in (0..out.len()).rev() {
        if let Some(p) = out[i].parent {
            let c = out[i].clone();
            let p = &mut out[p as usize];
            p.allocated = add(p.allocated, c.allocated)?;
            p.apparent = add(p.apparent, c.apparent)?;
            p.items = add(p.items, add(c.items, 1)?)?;
            p.errors = add(p.errors, c.errors)?;
            p.min_scanned_at = p.min_scanned_at.min(c.min_scanned_at);
            p.newest_mtime = p.newest_mtime.max(c.newest_mtime);
        }
    }
    Ok(())
}

fn roots(roots: Vec<crate::scan::RootInfo>, raw_ids: &HashMap<u32, i64>) -> Vec<Root> {
    roots
        .into_iter()
        .enumerate()
        .map(|(raw, r)| Root {
            entry_id: raw_ids[&(raw as u32)],
            ordinal: raw as i64,
            path: r.path.as_os_str().as_bytes().to_vec(),
            dev: r.dev,
            fs_type: r.fs_type,
            stats: r.stats,
        })
        .collect()
}

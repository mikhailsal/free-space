//! Only the inode groups touched by the old/new subtree participate in refresh.
use super::{Entry, schema::COLUMNS};
use crate::{Result, scan::flags};
use rusqlite::{Connection, params};
use std::collections::{HashMap, HashSet};
type Inode = (i64, i64);
fn inode(e: &Entry) -> Option<Inode> {
    e.dev.zip(e.ino)
}
fn charge(e: &mut Entry, owner: bool) {
    if owner {
        e.flags &= !flags::ALIAS;
        e.allocated = e.own_allocated;
        e.apparent = e.own_apparent;
    } else {
        e.flags |= flags::ALIAS;
        e.allocated = 0;
        e.apparent = 0;
    }
}
pub(super) fn scan(entries: &mut [Entry]) {
    let mut winners: HashMap<Inode, usize> = HashMap::new();
    for (i, e) in entries.iter().enumerate() {
        if let Some(k) = inode(e) {
            let w = winners.entry(k).or_insert(i);
            if e.path < entries[*w].path {
                *w = i;
            }
        }
    }
    for (i, e) in entries.iter_mut().enumerate() {
        if let Some(k) = inode(e) {
            charge(e, winners[&k] == i);
        }
    }
}

/// Changed external aliases carry their old entry alongside their replacement.
pub(super) struct Changes {
    pub entries: Vec<(Entry, Entry)>,
    pub groups: usize,
    pub members: usize,
}
pub(super) fn refresh(conn: &Connection, old: &[Entry], new: &mut [Entry]) -> Result<Changes> {
    let removed: HashSet<_> = old.iter().map(|e| e.id).collect();
    let mut groups: HashMap<Inode, Vec<usize>> = HashMap::new();
    for e in old {
        if let Some(k) = inode(e) {
            groups.entry(k).or_default();
        }
    }
    for (i, e) in new.iter().enumerate() {
        if let Some(k) = inode(e) {
            groups.entry(k).or_default().push(i);
        }
    }
    let mut changes = Changes {
        entries: Vec::new(),
        groups: groups.len(),
        members: 0,
    };
    let mut q = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM live WHERE dev=?1 AND ino=?2"
    ))?;
    for (k, indices) in groups {
        let known = q
            .query_map(params![k.0, k.1], Entry::row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        changes.members += known.len();
        let outside: Vec<_> = known.iter().filter(|e| !removed.contains(&e.id)).collect();
        let owner = known.iter().find(|e| e.flags & flags::ALIAS == 0);
        // This is v1's refresh policy: keep an outside owner or a replaced
        // owner's returning path. With no such owner, incomplete groups zero.
        let forced = if outside.is_empty() { None } else { owner };
        let forced_id = forced.filter(|e| !removed.contains(&e.id)).map(|e| e.id);
        let forced_path = forced
            .filter(|e| removed.contains(&e.id))
            .map(|e| e.path.as_slice());
        let nlink = outside
            .iter()
            .map(|e| e.nlink)
            .chain(indices.iter().map(|i| new[*i].nlink))
            .max()
            .unwrap_or(0);
        let zeroed = forced.is_none() && ((outside.len() + indices.len()) as i64) < nlink;
        // Full raw-path ordering differs from component-wise DFS ordering.
        let candidate = outside
            .iter()
            .map(|e| (*e, None))
            .chain(indices.iter().map(|i| (&new[*i], Some(*i))))
            .min_by(|(a, _), (b, _)| a.path.cmp(&b.path));
        let fallback = candidate.map(|(e, i)| (e.id, i));
        let returned =
            forced_path.and_then(|p| indices.iter().copied().find(|i| new[*i].path == p));
        for e in outside {
            let winner = !zeroed
                && match (forced_id, returned, fallback) {
                    (Some(id), _, _) => e.id == id,
                    (_, Some(_), _) => false,
                    (_, _, Some((id, None))) => e.id == id,
                    _ => false,
                };
            let mut updated = e.clone();
            charge(&mut updated, winner);
            if updated != *e {
                changes.entries.push((e.clone(), updated));
            }
        }
        for i in indices {
            let winner = !zeroed
                && match (forced_id, returned, fallback) {
                    (Some(_), _, _) => false,
                    (_, Some(index), _) => index == i,
                    (_, _, Some((_, Some(index)))) => index == i,
                    _ => false,
                };
            charge(&mut new[i], winner);
        }
    }
    Ok(changes)
}

//! Disk-backed refresh hardlink policy and live DFS ordering.
use super::*;
use crate::queries::common::Paths;
use crate::scan::tree::push_component;

struct Frame {
    id: i64,
    cursor: Option<(Vec<u8>, i64)>,
    path_len: usize,
}

pub(super) fn prepare(
    stage: &StagedTree,
    source: &Connection,
    target: i64,
    end: i64,
    guard: &mut SpaceGuard,
) -> Result<()> {
    stage.conn.execute_batch("CREATE TABLE live(id INTEGER PRIMARY KEY, ordinal INTEGER UNIQUE NOT NULL);
        CREATE TABLE winners(dev INTEGER NOT NULL, ino INTEGER NOT NULL, id INTEGER, PRIMARY KEY(dev,ino));")?;
    let mut ordinal = 0i64;
    for root in 0..stage.roots.roots.len() {
        visit_root(stage, root as i64, &mut ordinal, guard)?;
    }
    if ordinal as u64 != stage.count {
        return Err(Error::generic(
            "staging traversal did not visit every entry",
        ));
    }
    choose_groups(stage, source, target, end, guard)
}

fn enter(stage: &StagedTree, id: i64, path: &[u8], root: i64, ordinal: &mut i64) -> Result<Frame> {
    stage
        .conn
        .prepare_cached("INSERT INTO live VALUES (?1,?2)")?
        .execute(params![id, *ordinal])?;
    *ordinal += 1;
    stage
        .conn
        .prepare_cached("UPDATE links SET path=?2,root=?3 WHERE id=?1")?
        .execute(params![id, path, root])?;
    Ok(Frame {
        id,
        cursor: None,
        path_len: path.len(),
    })
}

fn visit_root(
    stage: &StagedTree,
    root: i64,
    ordinal: &mut i64,
    guard: &mut SpaceGuard,
) -> Result<()> {
    let mut path: Vec<u8> = stage
        .conn
        .prepare_cached("SELECT name FROM raw WHERE id=?1")?
        .query_row([root], |r| r.get(0))?;
    let mut stack = vec![enter(stage, root, &path, root, ordinal)?];
    while let Some(parent) = stack.last_mut() {
        guard.tick()?;
        let child: Option<(i64, Vec<u8>)> = if let Some((name, id)) = &parent.cursor {
            stage.conn.prepare_cached("SELECT id,name FROM raw WHERE parent=?1 AND (name,id)>(?2,?3) ORDER BY name,id LIMIT 1")?.query_row(params![parent.id,name,id],|r|Ok((r.get(0)?,r.get(1)?))).optional()?
        } else {
            stage
                .conn
                .prepare_cached("SELECT id,name FROM raw WHERE parent=?1 ORDER BY name,id LIMIT 1")?
                .query_row([parent.id], |r| Ok((r.get(0)?, r.get(1)?)))
                .optional()?
        };
        if let Some((id, name)) = child {
            parent.cursor = Some((name.clone(), id));
            path.truncate(parent.path_len);
            push_component(&mut path, &name);
            stack.push(enter(stage, id, &path, root, ordinal)?);
        } else {
            stack.pop();
        }
    }
    Ok(())
}

fn choose_groups(
    stage: &StagedTree,
    source: &Connection,
    target: i64,
    end: i64,
    guard: &mut SpaceGuard,
) -> Result<()> {
    let mut keys = stage
        .conn
        .prepare_cached("SELECT dev,ino FROM link_keys ORDER BY dev,ino")?;
    let mut rows = keys.query([])?;
    while let Some(row) = rows.next()? {
        let (dev, ino): (i64, i64) = (row.get(0)?, row.get(1)?);
        let winner = choose(stage, source, target, end, dev, ino, guard)?;
        stage
            .conn
            .prepare_cached("INSERT INTO winners VALUES (?1,?2,?3)")?
            .execute(params![dev, ino, winner])?;
        guard.tick()?;
    }
    Ok(())
}

fn choose(
    stage: &StagedTree,
    source: &Connection,
    target: i64,
    end: i64,
    dev: i64,
    ino: i64,
    guard: &mut SpaceGuard,
) -> Result<Option<i64>> {
    let (outside, owner, known, max_nlink) = group_facts(stage, target, end, dev, ino, guard)?;
    let mut forced_id = None;
    let mut forced_path = None;
    if outside > 0
        && let Some(old) = owner
    {
        if old > target && old <= end {
            forced_path = Some(Paths::new(source).path(old)?);
        } else {
            forced_id = Some(
                stage
                    .conn
                    .prepare_cached("SELECT stage_id FROM old_map WHERE old_id=?1")?
                    .query_row([old], |r| r.get::<_, i64>(0))?,
            );
        }
    } else if known < u64::from(max_nlink) {
        return Ok(None);
    }
    if let Some(id) = forced_id {
        let live: bool = stage
            .conn
            .prepare_cached("SELECT EXISTS(SELECT 1 FROM live WHERE id=?1)")?
            .query_row([id], |r| r.get(0))?;
        if live {
            return Ok(Some(id));
        }
    }
    choose_live(stage, dev, ino, forced_path.as_deref(), guard)
}

fn choose_live(
    stage: &StagedTree,
    dev: i64,
    ino: i64,
    forced: Option<&[u8]>,
    guard: &mut SpaceGuard,
) -> Result<Option<i64>> {
    let mut stmt = stage
        .conn
        .prepare_cached("SELECT id,path FROM links WHERE dev=?1 AND ino=?2")?;
    let mut rows = stmt.query(params![dev, ino])?;
    let mut best: Option<(Vec<u8>, i64, i64)> = None;
    let mut forced_best: Option<(i64, i64)> = None;
    while let Some(row) = rows.next()? {
        let id: i64 = row.get(0)?;
        let path: Vec<u8> = row.get(1)?;
        let ordinal: Option<i64> = stage
            .conn
            .prepare_cached("SELECT ordinal FROM live WHERE id=?1")?
            .query_row([id], |r| r.get(0))
            .optional()?;
        if let Some(ordinal) = ordinal {
            if forced == Some(path.as_slice())
                && forced_best.is_none_or(|(prior, _)| ordinal > prior)
            {
                forced_best = Some((ordinal, id));
            }
            if best.as_ref().is_none_or(|(prior, order, _)| {
                path.as_slice() < prior.as_slice() || (path == *prior && ordinal < *order)
            }) {
                best = Some((path, ordinal, id));
            }
        }
        guard.tick()?;
    }
    Ok(forced_best
        .map(|(_, id)| id)
        .or_else(|| best.map(|(_, _, id)| id)))
}

fn group_facts(
    stage: &StagedTree,
    target: i64,
    end: i64,
    dev: i64,
    ino: i64,
    guard: &mut SpaceGuard,
) -> Result<(u64, Option<i64>, u64, u32)> {
    let mut outside = 0u64;
    let mut owner = None;
    let mut max_nlink = 0u32;
    let mut old = stage.conn.prepare_cached(
        "SELECT l.old_id,l.was_owner,coalesce(r.nlink,o.nlink) FROM old_link l
         JOIN old_map m ON m.old_id=l.old_id JOIN old_raw o ON o.old_id=l.old_id
         LEFT JOIN raw r ON r.id=m.stage_id
         WHERE l.dev=?1 AND l.ino=?2 ORDER BY l.seq",
    )?;
    let mut rows = old.query(params![dev, ino])?;
    while let Some(row) = rows.next()? {
        let id: i64 = row.get(0)?;
        let was_owner: bool = row.get(1)?;
        if owner.is_none() && was_owner {
            owner = Some(id);
        }
        if id <= target || id > end {
            outside += 1;
            max_nlink = max_nlink.max(row.get::<_, i64>(2)? as u32);
        }
        guard.tick()?;
    }
    let mut known = outside;
    let mut new = stage.conn.prepare_cached(
        "SELECT r.nlink FROM new_link l JOIN raw r ON r.id=l.id WHERE l.dev=?1 AND l.ino=?2",
    )?;
    let mut rows = new.query(params![dev, ino])?;
    while let Some(row) = rows.next()? {
        known += 1;
        max_nlink = max_nlink.max(row.get::<_, i64>(0)? as u32);
        guard.tick()?;
    }
    Ok((outside, owner, known, max_nlink))
}

#[cfg(test)]
#[path = "policy/tests.rs"]
mod tests;

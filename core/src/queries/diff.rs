//! `diff`: what changed since the previous generation (D6). Opens the current
//! snapshot first, then prev, and checks the pairing (exit 3/4/5). Rows are
//! matched by `path_hash`; returned pairs are verified by name and parent hash
//! (exit 7 on a collision).

use std::collections::HashMap;

use rusqlite::{Connection, params};

use super::common::{COLS, Paths, Row, find_path, header, lossy, normalize, raw_if_lossy};
use super::params::{DiffParams, KindFilter, MAX_DEPTH, PathArg, check_limit};
use super::types::{DiffEntry, DiffResponse, DiffStatus, Refreshed};
use crate::error::{Error, ErrorCode, Result};
use crate::scan::Kind;
use crate::snapshot::SnapshotDir;
use crate::snapshot::reader::Snapshot;

pub fn no_prev(msg: &str) -> Error {
    Error::new(ErrorCode::NoPrev, format!("no previous generation: {msg}"))
        .with_fix("wait for the next scheduled scan (needs keep_previous = true in /etc/freespace/config.toml)")
}

/// Open current, then prev, and validate the pair.
pub fn open_pair(dir: &SnapshotDir) -> Result<(Snapshot, Snapshot)> {
    let cur = Snapshot::open_current(dir)?;
    let prev_path = dir.prev();
    let prev = match Snapshot::open(&prev_path) {
        Ok(p) => p,
        Err(e) if e.code == ErrorCode::NoSnapshot => {
            return Err(no_prev("snapshot.prev.db does not exist"));
        }
        // left from before an upgrade: there is nothing to compare with yet
        Err(e) if e.code == ErrorCode::Schema => {
            return Err(no_prev(
                "snapshot.prev.db is from another freespace version",
            ));
        }
        Err(e) => return Err(e),
    };
    if prev.meta.generation == cur.meta.generation {
        return Err(no_prev("prev and current are the same generation"));
    }
    if Some(prev.meta.generation) != cur.meta.prev_generation {
        return Err(Error::new(
            ErrorCode::Rotated,
            format!(
                "snapshot rotated: current generation {} expects prev {:?}, found {}",
                cur.meta.generation, cur.meta.prev_generation, prev.meta.generation
            ),
        )
        .with_fix("retry the same command"));
    }
    Ok((cur, prev))
}

pub(crate) struct LevelRow {
    pub(crate) row: Row,
    pub(crate) hash: i64,
}

pub(crate) fn level(
    conn: &Connection,
    x: i64,
    depth: u32,
    kind: KindFilter,
) -> Result<Vec<LevelRow>> {
    let kind_sql = match kind {
        KindFilter::File => "AND kind = 0",
        KindFilter::Dir => "AND kind = 1",
        KindFilter::Any => "",
    };
    let sql = if depth == 1 {
        format!("SELECT {COLS}, path_hash FROM entries WHERE parent_id = ?1 {kind_sql} AND ?2 = 1")
    } else {
        format!(
            "WITH RECURSIVE lvl(id, d) AS (
               SELECT id, 1 FROM entries WHERE parent_id = ?1
               UNION ALL
               SELECT e.id, lvl.d + 1 FROM entries e JOIN lvl ON e.parent_id = lvl.id WHERE lvl.d < ?2
             )
             SELECT {COLS}, path_hash FROM entries WHERE id IN (SELECT id FROM lvl WHERE d = ?2) {kind_sql}"
        )
    };
    let mut st = conn.prepare_cached(&sql)?;
    let rows = st
        .query_map(params![x, depth], |r| {
            Ok(LevelRow {
                row: Row::from_sql(r)?,
                hash: r.get(15)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

fn parent_hash(conn: &Connection, id: Option<i64>) -> Result<Option<i64>> {
    match id {
        None => Ok(None),
        Some(p) => Ok(Some(
            conn.prepare_cached("SELECT path_hash FROM entries WHERE id = ?1")?
                .query_row(params![p], |r| r.get(0))?,
        )),
    }
}

/// prev-side id ranges that lie under directories the current scan could not read.
fn denied_prev_ranges(cur: &Snapshot, prev: &Snapshot, x: &Row) -> Result<Vec<(i64, i64)>> {
    let mut st = cur.conn.prepare_cached(
        "SELECT d.entry_id FROM denied d WHERE d.entry_id BETWEEN ?1 AND ?2 ORDER BY d.entry_id",
    )?;
    let ids = st
        .query_map(params![x.id, x.subtree_end], |r| r.get::<_, i64>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut paths = Paths::new(&cur.conn);
    let mut out = Vec::new();
    for id in ids {
        let p = paths.path(id)?;
        if let Some(pr) = find_path(&prev.conn, &p)? {
            out.push((pr.id, pr.subtree_end));
        }
    }
    Ok(out)
}

fn refreshes(conn: &Connection) -> Result<Vec<Refreshed>> {
    let mut st = conn
        .prepare_cached("SELECT path, scanned_at, generation FROM refreshes ORDER BY scanned_at")?;
    let v = st
        .query_map([], |r| {
            Ok(Refreshed {
                path: lossy(&r.get::<_, Vec<u8>>(0)?),
                scanned_at: r.get(1)?,
                generation: r.get::<_, i64>(2)? as u64,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(v)
}

pub fn refreshed(snap: &Snapshot) -> Result<Vec<Refreshed>> {
    refreshes(&snap.conn)
}

fn root_paths(conn: &Connection) -> Result<Vec<Vec<u8>>> {
    let mut st = conn.prepare_cached("SELECT path FROM roots ORDER BY path")?;
    let v = st
        .query_map([], |r| r.get::<_, Vec<u8>>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(v)
}

pub fn diff(dir: &SnapshotDir, p: &DiffParams) -> Result<DiffResponse> {
    let depth = p.depth.unwrap_or(1);
    if depth == 0 || depth > MAX_DEPTH {
        return Err(Error::usage(format!(
            "depth must be between 1 and {MAX_DEPTH}"
        )));
    }
    let kind = match &p.kind {
        Some(k) => KindFilter::parse(k)?,
        None => KindFilter::Dir,
    };
    let limit = check_limit(p.limit)?;
    let input = normalize(&PathArg::new(p.path.clone(), p.path_raw.clone()).bytes()?)?;
    let (cur, prev) = open_pair(dir)?;
    diff_pair(&cur, &prev, &input, depth, kind, limit)
}

pub fn diff_pair(
    cur: &Snapshot,
    prev: &Snapshot,
    path: &[u8],
    depth: u32,
    kind: KindFilter,
    limit: u32,
) -> Result<DiffResponse> {
    let xc = find_path(&cur.conn, path)?;
    let xp = find_path(&prev.conn, path)?;
    if xc.is_none() && xp.is_none() {
        return Err(Error::new(
            ErrorCode::PathNotFound,
            format!(
                "{} is in neither the current nor the previous snapshot",
                lossy(path)
            ),
        )
        .with_fix("freespace ls /"));
    }
    let cur_rows = match &xc {
        Some(x) => level(&cur.conn, x.id, depth, kind)?,
        None => Vec::new(),
    };
    let prev_rows = match &xp {
        Some(x) => level(&prev.conn, x.id, depth, kind)?,
        None => Vec::new(),
    };
    let denied = match &xc {
        Some(x) => denied_prev_ranges(cur, prev, x)?,
        None => Vec::new(),
    };

    let mut prev_by_hash: HashMap<i64, LevelRow> =
        prev_rows.into_iter().map(|r| (r.hash, r)).collect();
    // (current row, prev row)
    let mut pairs: Vec<(Option<LevelRow>, Option<LevelRow>)> = Vec::new();
    for c in cur_rows {
        let pm = prev_by_hash.remove(&c.hash);
        pairs.push((Some(c), pm));
    }
    for (_, pr) in prev_by_hash {
        if denied
            .iter()
            .any(|&(a, b)| pr.row.id >= a && pr.row.id <= b)
        {
            continue; // not observed this time, not gone (OV-3)
        }
        pairs.push((None, Some(pr)));
    }
    let delta_of = |c: &Option<LevelRow>, p: &Option<LevelRow>| {
        c.as_ref().map(|r| r.row.allocated).unwrap_or(0)
            - p.as_ref().map(|r| r.row.allocated).unwrap_or(0)
    };
    pairs.sort_by(|a, b| {
        let (da, db) = (delta_of(&a.0, &a.1), delta_of(&b.0, &b.1));
        db.unsigned_abs()
            .cmp(&da.unsigned_abs())
            .then((db > 0).cmp(&(da > 0)))
            .then_with(|| {
                let na = a.0.as_ref().or(a.1.as_ref()).map(|r| &r.row.name);
                let nb = b.0.as_ref().or(b.1.as_ref()).map(|r| &r.row.name);
                na.cmp(&nb)
            })
            .then_with(|| {
                // deterministic: current rows by id, then gone rows by their prev id
                let key = |x: &(Option<LevelRow>, Option<LevelRow>)| match (&x.0, &x.1) {
                    (Some(c), _) => (0, c.row.id),
                    (None, Some(p)) => (1, p.row.id),
                    (None, None) => (2, 0),
                };
                key(a).cmp(&key(b))
            })
    });
    let total_count = pairs.len() as u64;
    pairs.truncate(limit as usize);

    let mut cpaths = Paths::new(&cur.conn);
    let mut ppaths = Paths::new(&prev.conn);
    let mut entries = Vec::with_capacity(pairs.len());
    for (c, pv) in &pairs {
        if let (Some(c), Some(pv)) = (c, pv) {
            let same_parent = parent_hash(&cur.conn, c.row.parent_id)?
                == parent_hash(&prev.conn, pv.row.parent_id)?;
            if c.row.name != pv.row.name || !same_parent {
                return Err(Error::new(
                    ErrorCode::HashCollision,
                    format!(
                        "path hash collision between {} (current) and {} (prev)",
                        lossy(&cpaths.path(c.row.id)?),
                        lossy(&ppaths.path(pv.row.id)?)
                    ),
                )
                .with_fix("report this at https://github.com/mikhailsal/free-space/issues; ls and top are unaffected"));
            }
        }
        let (src, paths) = match c {
            Some(c) => (&c.row, &mut cpaths),
            None => (&pv.as_ref().expect("one side present").row, &mut ppaths),
        };
        let full = paths.path(src.id)?;
        let allocated = c
            .as_ref()
            .map(|r| r.row.allocated.max(0) as u64)
            .unwrap_or(0);
        let allocated_prev = pv
            .as_ref()
            .map(|r| r.row.allocated.max(0) as u64)
            .unwrap_or(0);
        let items = c.as_ref().map(|r| r.row.items.max(0) as u64).unwrap_or(0);
        let items_prev = pv.as_ref().map(|r| r.row.items.max(0) as u64).unwrap_or(0);
        let delta = allocated as i64 - allocated_prev as i64;
        let status = match (c, pv) {
            (Some(_), None) => DiffStatus::New,
            (None, Some(_)) => DiffStatus::Gone,
            _ if delta != 0 || items != items_prev => DiffStatus::Changed,
            _ => DiffStatus::Same,
        };
        entries.push(DiffEntry {
            path: lossy(&full),
            path_raw: raw_if_lossy(&full),
            name: lossy(&src.name),
            kind: Kind::from_code(src.kind),
            status,
            allocated,
            allocated_prev,
            delta,
            items,
            items_prev,
            mtime: src.mtime,
            newest_mtime: src.newest_mtime,
        });
    }

    let refreshed = refreshes(&cur.conn)?;
    let roots_changed = root_paths(&cur.conn)? != root_paths(&prev.conn)?;
    let x_delta = xc.as_ref().map(|r| r.allocated).unwrap_or(0)
        - xp.as_ref().map(|r| r.allocated).unwrap_or(0);
    Ok(DiffResponse {
        header: header(cur, xc.as_ref())?,
        path: lossy(path),
        path_raw: raw_if_lossy(path),
        prev_scanned_at: prev.meta.scanned_at,
        depth,
        kind: kind.as_str().into(),
        partial: !refreshed.is_empty(),
        refreshed,
        roots_changed,
        delta: x_delta,
        truncated: total_count > entries.len() as u64,
        entries,
        total_count,
        limit,
    })
}

/// Status sentence clause "+9 G since prev in /home/tass/Downloads": descend
/// from `start` while one child directory carries at least half of the growth.
pub fn top_growth(cur: &Snapshot, prev: &Snapshot, start: &[u8]) -> Result<Option<(String, i64)>> {
    let (Some(mut xc), Some(mut xp)) =
        (find_path(&cur.conn, start)?, find_path(&prev.conn, start)?)
    else {
        return Ok(None);
    };
    let mut path = start.to_vec();
    let mut delta = xc.allocated - xp.allocated;
    if delta <= 0 {
        return Ok(None);
    }
    for _ in 0..12 {
        let pc = level(&cur.conn, xc.id, 1, KindFilter::Dir)?;
        let pp: HashMap<i64, LevelRow> = level(&prev.conn, xp.id, 1, KindFilter::Dir)?
            .into_iter()
            .map(|r| (r.hash, r))
            .collect();
        let best = pc
            .into_iter()
            .map(|c| {
                let p = pp.get(&c.hash).map(|p| p.row.clone());
                (
                    c.row.allocated - p.as_ref().map_or(0, |p| p.allocated),
                    c,
                    p,
                )
            })
            .max_by_key(|(d, _, _)| *d);
        match best {
            Some((d, c, p)) if d > 0 && d * 2 >= delta => {
                crate::scan::tree::push_component(&mut path, &c.row.name);
                delta = d;
                // A folder new since prev: nothing to compare below it.
                let Some(p) = p else { break };
                xc = c.row;
                xp = p;
            }
            _ => break,
        }
    }
    Ok(Some((lossy(&path), delta)))
}

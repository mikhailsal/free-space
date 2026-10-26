use rusqlite::{OptionalExtension, params};

use crate::queries::common::{lossy, normalize, raw_if_lossy, shell_path_args};
use crate::queries::params::PathArg;
use crate::queries::types::{EntryOut, Header, MountOut, Refreshed};
use crate::scan::{Kind, flags};
use crate::snapshot::v2::{Entry, Reader};
use crate::{Error, ErrorCode, Result};

pub(super) fn input(path: &Option<String>, raw: &Option<String>) -> Result<Vec<u8>> {
    normalize(&PathArg::new(path.clone(), raw.clone()).bytes()?)
}

pub(super) fn under(root: &[u8], path: &[u8]) -> bool {
    root == b"/" || root == path || (path.starts_with(root) && path.get(root.len()) == Some(&b'/'))
}

pub(super) fn get_path(reader: &Reader, path: &[u8], previous: bool) -> Result<Option<Entry>> {
    // NUL can arrive through --path-raw but cannot name a filesystem entry.
    // Public v1 queries report PATH_NOT_FOUND, not the store's validation error.
    if path.contains(&0) {
        return Ok(None);
    }
    reader.get(path, previous)
}

pub(super) fn resolve(reader: &Reader, path: &[u8]) -> Result<Entry> {
    if let Some(entry) = get_path(reader, path, false)? {
        return Ok(entry);
    }
    let roots = reader.roots(false)?;
    let shown = lossy(path);
    if !roots.iter().any(|r| under(&r.path, path)) {
        let names: Vec<_> = roots.iter().map(|r| lossy(&r.path)).collect();
        return Err(Error::new(
            ErrorCode::PathNotFound,
            format!(
                "{shown} is outside the scanned roots ({})",
                names.join(", ")
            ),
        )
        .with_fix(format!("use a path under {}", names.join(" or "))));
    }
    let mut ancestor = path.to_vec();
    while let Some(pos) = ancestor.iter().rposition(|b| *b == b'/') {
        ancestor.truncate(pos.max(1));
        if get_path(reader, &ancestor, false)?.is_some() || pos <= 1 {
            break;
        }
    }
    let meta = reader.meta(false)?;
    Err(Error::new(ErrorCode::PathNotFound, format!("{shown} is not in the snapshot from {} {} (created later, removed, or on another filesystem)", crate::format::local_date(meta.scanned_at), crate::format::local_hm(meta.scanned_at)))
        .with_fix(format!("freespace ls {}", shell_path_args(&ancestor))))
}

pub(super) fn check_snapshot(reader: &Reader, want: Option<&str>) -> Result<()> {
    let meta = reader.meta(false)?;
    if let Some(id) = want.filter(|id| !id.is_empty())
        && id != meta.snapshot_id
    {
        return Err(Error::new(
            ErrorCode::Rotated,
            format!(
                "snapshot {id} was replaced by {} (generation {})",
                meta.snapshot_id, meta.generation
            ),
        )
        .with_fix("reload: the new snapshot is ready"));
    }
    Ok(())
}

pub(super) fn header(reader: &Reader, focus: Option<&Entry>) -> Result<Header> {
    let meta = reader.meta(false)?;
    let (incomplete, unknown) = reader.unknown_subtrees(false, 100)?;
    Ok(Header {
        host: meta.host,
        os: meta.os,
        generation: meta.generation,
        prev_generation: meta.prev_generation,
        scanned_at: meta.scanned_at,
        age_seconds: (crate::platform::now_secs()
            - focus.map_or(meta.scanned_at, |e| e.min_scanned_at))
        .max(0),
        schema_version: meta.schema_version,
        snapshot_id: meta.snapshot_id,
        incomplete,
        unknown_subtrees: unknown.iter().map(|p| lossy(p)).collect(),
    })
}

pub(super) fn name(entry: &Entry) -> &[u8] {
    if entry.parent.is_none() {
        &entry.path
    } else {
        entry
            .path
            .rsplit(|b| *b == b'/')
            .next()
            .unwrap_or(&entry.path)
    }
}

pub(super) fn output(reader: &Reader, entry: &Entry) -> Result<EntryOut> {
    let mut out = EntryOut {
        path: lossy(&entry.path),
        path_raw: raw_if_lossy(&entry.path),
        name: lossy(name(entry)),
        kind: entry.kind,
        allocated: entry.allocated.max(0) as u64,
        apparent: entry.apparent.max(0) as u64,
        mtime: entry.mtime,
        newest_mtime: entry.newest_mtime.unwrap_or(0),
        nlink: entry.nlink.max(0) as u64,
        items: entry.items.max(0) as u64,
        errors: entry.errors.max(0) as u64,
        scanned_at: entry.scanned_at,
        denied: entry.flags & flags::DENIED != 0,
        hardlink_alias: entry.flags & flags::ALIAS != 0,
        hardlink_owner: None,
        mount: None,
    };
    if entry.kind == Kind::Mount
        && let Some(mount) = reader.mount(entry.id, false)?
    {
        out.mount = Some(MountOut {
            fs_type: mount.fs_type,
            used: mount.used.map(|v| v as u64),
            total: mount.total.map(|v| v as u64),
            root: reader
                .roots(false)?
                .into_iter()
                .find(|r| r.path == mount.path)
                .map(|r| lossy(&r.path)),
        });
    }
    // Resolve the inode from this exact entry, not path lookup: a mount and
    // independently configured root may have the same raw path.
    if entry.kind != Kind::Dir
        && entry.nlink > 1
        && let Some((dev, ino)) = entry.dev.zip(entry.ino)
    {
        let g = reader.generation(false)?;
        let owner: Option<Vec<u8>> = reader.connection().query_row(
            "SELECT path FROM versions WHERE dev=?1 AND ino=?2 AND (flags & 2)=0 AND born<=?3 AND (dead IS NULL OR dead>?3) LIMIT 1",
            params![dev, ino, g], |r| r.get(0)).optional()?;
        out.hardlink_owner = owner.map(|p| lossy(&p));
    }
    Ok(out)
}

pub(super) fn upper(key: &[u8]) -> Vec<u8> {
    if key.is_empty() {
        return vec![255];
    }
    let mut end = key.to_vec();
    *end.last_mut().expect("nonempty component key") += 1;
    end
}

pub(super) fn refreshed(reader: &Reader) -> Result<Vec<Refreshed>> {
    let mut rows: Vec<_> = reader
        .refresh_history()?
        .into_iter()
        .map(|(generation, path, scanned_at)| Refreshed {
            path: lossy(&path),
            scanned_at,
            generation: generation as u64,
        })
        .collect();
    rows.sort_by_key(|r| r.scanned_at);
    Ok(rows)
}

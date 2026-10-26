use rusqlite::params;

use super::{common::*, pass};
use crate::queries::common::{lossy, raw_if_lossy};
use crate::queries::glob;
use crate::queries::params::{
    FindParams, KindFilter, LsParams, SortKey, TopParams, check_limit, parse_size, parse_time_bound,
};
use crate::queries::pass::{Filter, Order};
use crate::queries::types::{FindEntry, FindResponse, LsResponse, TopResponse};
use crate::snapshot::v2::{ENTRY_COLUMNS, Entry, Reader};
use crate::{Error, Result};

pub fn ls(reader: &Reader, p: &LsParams) -> Result<LsResponse> {
    reader.with_pin(|reader| {
        check_snapshot(reader, p.snapshot.as_deref())?;
        let sort = p.sort.as_deref().map(SortKey::parse).transpose()?.unwrap_or_default();
        let limit = check_limit(p.limit)?;
        let path = input(&p.path, &p.path_raw)?;
        let entry = resolve(reader, &path)?;
        let generation = reader.generation(false)?;
        let total_count: i64 = reader.connection().query_row(
            "SELECT count(*) FROM versions WHERE parent=?1 AND born<=?2 AND (dead IS NULL OR dead>?2)",
            params![entry.id, generation], |r| r.get(0))?;
        let order = match sort {
            SortKey::Allocated => "allocated DESC, sort_key",
            SortKey::Apparent => "apparent DESC, sort_key",
            SortKey::Mtime => "mtime ASC, sort_key",
        };
        let mut query = reader.connection().prepare(&format!(
            "SELECT {ENTRY_COLUMNS} FROM versions WHERE parent=?1 AND born<=?2 AND (dead IS NULL OR dead>?2) ORDER BY {order} LIMIT ?3"
        ))?;
        let mut rows = query.query(params![entry.id, generation, limit])?;
        let mut entries = Vec::new();
        while let Some(row) = rows.next()? {
            entries.push(output(reader, &Entry::row(row)?)?);
        }
        Ok(LsResponse {
            header: header(reader, Some(&entry))?, entry: output(reader, &entry)?, entries,
            total_count: total_count as u64, truncated: total_count as u64 > limit as u64,
            limit, sort: sort.as_str().into(),
        })
    })
}

pub fn top(reader: &Reader, p: &TopParams) -> Result<TopResponse> {
    reader.with_pin(|reader| {
        let kind = p
            .kind
            .as_deref()
            .map(KindFilter::parse)
            .transpose()?
            .unwrap_or(KindFilter::File);
        if kind == KindFilter::Any {
            return Err(Error::usage("top takes --kind file or --kind dir"));
        }
        let sort = p
            .sort
            .as_deref()
            .map(SortKey::parse)
            .transpose()?
            .unwrap_or_default();
        let limit = check_limit(p.limit)?;
        let min_size = p.min_size.as_deref().map(parse_size).transpose()?;
        let older = p
            .older_than
            .as_deref()
            .map(|v| parse_time_bound(v, crate::platform::now_secs()))
            .transpose()?;
        let path = input(&p.path, &p.path_raw)?;
        let scope = resolve(reader, &path)?;
        let filter = Filter {
            kind,
            min_size: i64::try_from(min_size.unwrap_or(0)).unwrap_or(i64::MAX),
            older,
            newer: None,
            name: None,
        };
        let order = Order {
            sort,
            limit,
            list_nested: false,
        };
        let found = pass::run(reader, &scope, &filter, &order)?;
        let mut total = found.total;
        total.count = total.counted;
        let mut entries = Vec::with_capacity(found.page.len());
        for ranked in found.page {
            let entry = reader
                .get_id(ranked.id, false)?
                .ok_or_else(|| Error::generic("entry vanished"))?;
            entries.push(output(reader, &entry)?);
        }
        Ok(TopResponse {
            header: header(reader, Some(&scope))?,
            path: lossy(&path),
            path_raw: raw_if_lossy(&path),
            kind: kind.as_str().into(),
            min_size,
            older_than: older,
            sort: sort.as_str().into(),
            truncated: total.count > entries.len() as u64,
            entries,
            total_count: total.count,
            total_allocated: total.allocated,
            total,
            limit,
        })
    })
}

pub fn find(reader: &Reader, p: &FindParams) -> Result<FindResponse> {
    reader.with_pin(|reader| {
        check_snapshot(reader, p.snapshot.as_deref())?;
        let now = crate::platform::now_secs();
        let older = p
            .older_than
            .as_deref()
            .map(|v| parse_time_bound(v, now))
            .transpose()?;
        let newer = p
            .newer_than
            .as_deref()
            .map(|v| parse_time_bound(v, now))
            .transpose()?;
        let kind = p
            .kind
            .as_deref()
            .map(KindFilter::parse)
            .transpose()?
            .unwrap_or(if older.is_some() || newer.is_some() {
                KindFilter::File
            } else {
                KindFilter::Any
            });
        let sort = p
            .sort
            .as_deref()
            .map(SortKey::parse)
            .transpose()?
            .unwrap_or_default();
        if sort == SortKey::Apparent {
            return Err(Error::usage("find sorts by allocated or mtime"));
        }
        let limit = check_limit(p.limit)?;
        let min_size = p.min_size.as_deref().map(parse_size).transpose()?;
        let name = find_name(p)?;
        let path = input(&p.path, &p.path_raw)?;
        let scope = resolve(reader, &path)?;
        let filter = Filter {
            kind,
            min_size: i64::try_from(min_size.unwrap_or(0)).unwrap_or(i64::MAX),
            older,
            newer,
            name,
        };
        let order = Order {
            sort,
            limit,
            list_nested: true,
        };
        let pass::Pass { total, page } = pass::run(reader, &scope, &filter, &order)?;
        let mut entries = Vec::with_capacity(page.len());
        for ranked in page {
            let entry = reader
                .get_id(ranked.id, false)?
                .ok_or_else(|| Error::generic("entry vanished"))?;
            entries.push(FindEntry {
                entry: output(reader, &entry)?,
                nested: ranked.nested,
            });
        }
        Ok(FindResponse {
            header: header(reader, Some(&scope))?,
            path: lossy(&path),
            path_raw: raw_if_lossy(&path),
            kind: kind.as_str().into(),
            sort: sort.as_str().into(),
            truncated: total.count > entries.len() as u64,
            entries,
            total_count: total.count,
            total_allocated: total.allocated,
            total,
            limit,
        })
    })
}

fn find_name(p: &FindParams) -> Result<Option<(String, bool)>> {
    let name = match (&p.name, &p.iname) {
        (Some(_), Some(_)) => {
            return Err(Error::usage("give either --name or --iname, not both"));
        }
        (Some(n), None) => Some((n.clone(), false)),
        (None, Some(n)) => Some((n.clone(), true)),
        _ => None,
    };
    if let Some((pattern, _)) = &name {
        glob::validate(pattern)?;
    }
    Ok(name)
}

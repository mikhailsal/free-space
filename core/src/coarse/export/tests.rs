use std::path::Path;

use super::*;
use crate::coarse::merge::rebuild;
use crate::coarse::model::{BUCKET_ENTRIES, MountMeta};
use crate::coarse::store;
use crate::coarse::testing::{KIB, model_of, put, sample, snapshot_of};
use crate::platform::FsStats;
use crate::snapshot::reader::Snapshot;

const THRESHOLD: u64 = 100 * KIB;

struct Row {
    id: i64,
    parent: Option<i64>,
    name: String,
    kind: i64,
    flags: i64,
    allocated: i64,
    items: i64,
    end: i64,
}

fn rows(snap: &Snapshot) -> Vec<Row> {
    let mut st = snap
        .conn
        .prepare("SELECT id, parent_id, name, kind, flags, allocated, items, subtree_end FROM entries ORDER BY id")
        .unwrap();
    st.query_map([], |r| {
        let name: Vec<u8> = r.get(2)?;
        Ok(Row {
            id: r.get(0)?,
            parent: r.get(1)?,
            name: String::from_utf8_lossy(&name).into_owned(),
            kind: r.get(3)?,
            flags: r.get(4)?,
            allocated: r.get(5)?,
            items: r.get(6)?,
            end: r.get(7)?,
        })
    })
    .unwrap()
    .map(Result::unwrap)
    .collect()
}

fn descends(rows: &[Row], mut id: i64, from: i64) -> bool {
    while let Some(parent) = rows[id as usize].parent {
        if parent == from {
            return true;
        }
        id = parent;
    }
    false
}

fn export(model: &Model, dir: &Path) -> Snapshot {
    snapshot_of(model, &dir.join("coarse.db"))
}

#[test]
fn subtree_ranges_follow_the_rows_below_and_not_the_item_counts() {
    let tmp = tempfile::tempdir().unwrap();
    let top = tmp.path().join("tree");
    sample(&top);
    let model = model_of(&top, THRESHOLD);
    let snap = export(&model, tmp.path());
    let rows = rows(&snap);
    assert!(rows.len() >= 6);
    for (i, row) in rows.iter().enumerate() {
        assert_eq!(row.id, i as i64, "ids are contiguous");
        assert!(row.end >= row.id && (row.end as usize) < rows.len());
        for other in &rows {
            let inside = other.id > row.id && other.id <= row.end;
            assert_eq!(
                inside,
                descends(&rows, other.id, row.id),
                "{} vs {}",
                row.name,
                other.name
            );
        }
    }
    // the item count is the number of entries found on disk, which is more than the rows
    let root = &rows[0];
    assert_eq!(root.items as u64, model.entries() - 1);
    assert!(root.end < root.items);
}

#[test]
fn buckets_become_pseudo_entries_carrying_the_folded_totals() {
    let tmp = tempfile::tempdir().unwrap();
    let top = tmp.path().join("tree");
    sample(&top);
    let model = model_of(&top, THRESHOLD);
    let snap = export(&model, tmp.path());
    let rows = rows(&snap);
    let pseudo: Vec<&Row> = rows.iter().filter(|r| r.flags & 8 != 0).collect();
    assert_eq!(pseudo.len(), 2, "one for the root and one for `a`");
    for p in &pseudo {
        assert_eq!(p.kind, 3);
        assert_eq!(p.name, "[small items]");
        assert_eq!(p.end, p.id, "a pseudo entry has nothing below it");
    }
    let root = &model.nodes.iter().find(|n| n.root).unwrap();
    let folded: i64 = pseudo
        .iter()
        .filter(|p| p.parent == Some(0))
        .map(|p| p.items)
        .sum();
    assert_eq!(folded as u64, root.rest().entries);
    // a directory's stored size is its own bytes plus everything its rows hold
    let kids: i64 = rows
        .iter()
        .filter(|r| r.parent == Some(0))
        .map(|r| r.allocated)
        .sum();
    assert_eq!(rows[0].allocated, kids + root.own_allocated as i64);
}

#[test]
fn the_side_tables_list_every_bucket_and_name_its_entry() {
    let tmp = tempfile::tempdir().unwrap();
    let top = tmp.path().join("tree");
    sample(&top);
    let model = model_of(&top, THRESHOLD);
    let built = build(&model, 1).unwrap();
    let dirs = model.nodes.iter().filter(|n| n.is_dir()).count();
    assert_eq!(
        built.side.buckets.len(),
        model.nodes.iter().map(|n| n.buckets.len()).sum::<usize>()
    );
    assert!(built.side.buckets.len() >= dirs);
    for row in &built.side.buckets {
        assert_eq!(row.entry_id.is_some(), row.bucket.stats.entries > 0);
    }
    assert_eq!(built.side.roots.len(), 1);
    assert_eq!(built.side.roots[0].entry_id, 0);
    assert_eq!(built.side.roots[0].threshold, THRESHOLD);

    let snap = export(&model, tmp.path());
    let side = store::read(&snap.conn).unwrap();
    assert_eq!(side.buckets.len(), built.side.buckets.len());
    assert_eq!(side.ppm, 1000);
    assert_eq!(store::folded_below(&snap.conn), Some(THRESHOLD));
}

#[test]
fn a_folder_with_several_buckets_names_them_by_range() {
    let tmp = tempfile::tempdir().unwrap();
    let top = tmp.path().join("tree");
    for i in 0..=BUCKET_ENTRIES {
        put(&top, &format!("many/f{i}"), 0);
    }
    let model = model_of(&top, 1024 * 1024 * KIB);
    let snap = export(&model, tmp.path());
    let names: Vec<String> = rows(&snap)
        .into_iter()
        .filter(|r| r.flags & 8 != 0)
        .map(|r| r.name)
        .collect();
    assert!(names.len() > 1);
    assert!(
        names
            .iter()
            .all(|n| n.starts_with("[small items ") && n.contains('-'))
    );
    // rows of one folder are sorted by name, pseudo entries included
    let rows = rows(&snap);
    let many = rows.iter().find(|r| r.name == "many").unwrap();
    let kids: Vec<&str> = rows
        .iter()
        .filter(|r| r.parent == Some(many.id))
        .map(|r| r.name.as_str())
        .collect();
    let mut sorted = kids.clone();
    sorted.sort_by_key(|n| n.as_bytes().to_vec());
    assert_eq!(kids, sorted);
}

#[test]
fn unreadable_folders_and_mounts_are_exported_with_their_tables() {
    let tmp = tempfile::tempdir().unwrap();
    let top = tmp.path().join("tree");
    sample(&top);
    let mut model = model_of(&top, THRESHOLD);
    let a = model
        .nodes
        .iter_mut()
        .find(|n| n.path == top.join("a"))
        .unwrap();
    a.flags |= flags::DENIED;
    a.own_errors = 1;
    let mut mount = a.clone();
    mount.path = top.join("mnt");
    mount.kind = Kind::Mount;
    mount.flags = 0;
    mount.own_errors = 0;
    mount.own_allocated = 0;
    mount.own_apparent = 0;
    mount.buckets = Vec::new();
    mount.mount = Some(Box::new(MountMeta {
        fs_type: "ext4".into(),
        stats: Some(FsStats {
            total: 1000,
            used: 400,
            ..FsStats::default()
        }),
    }));
    model.nodes.push(mount);
    rebuild(&mut model.nodes).unwrap();

    let built = build(&model, 1).unwrap();
    assert_eq!(built.denied, [top.join("a").display().to_string()]);
    assert_eq!(built.fin.denied.len(), 1);
    assert_eq!(built.tree.mounts.len(), 1);
    assert_eq!(built.tree.mounts[0].fs_type, "ext4");

    let snap = export(&model, tmp.path());
    let n: i64 = snap
        .conn
        .query_row("SELECT COUNT(*) FROM denied", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 1);
    let n: i64 = snap
        .conn
        .query_row(
            "SELECT COUNT(*) FROM mounts WHERE fs_type = 'ext4'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 1);
}

//! Public response parity for the opt-in adapter, including post-refresh IDs.

use std::fs::{self, File};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::time::{Duration, UNIX_EPOCH};

use freespace_core::ErrorCode;
use freespace_core::queries::{self, params::*, v2};
use freespace_core::snapshot::{
    SnapshotDir,
    reader::Snapshot,
    v2::{Reader, Store},
};
use serde::Serialize;
use serde_json::Value;

mod common;
use common::{Fx, fx};

fn store(f: &Fx) -> (Store, std::path::PathBuf) {
    let path = f.dir.path.with_extension("v2.db");
    (Store::open(&path).unwrap(), path)
}

fn scan(f: &Fx, store: &mut Store) {
    f.scan_roots(vec![f.root.clone()], 1, true).unwrap();
    store.scan(&f.root, 1).unwrap();
}

fn old_file(f: &Fx, path: &str, size: usize) {
    f.write_file(path, size);
    File::open(f.p(path))
        .unwrap()
        .set_modified(UNIX_EPOCH + Duration::from_secs(1_000_000_000))
        .unwrap();
}

fn comparable<T: Serialize>(value: T) -> Value {
    fn normalize(v: &mut Value) {
        match v {
            Value::Object(fields) => {
                // The two independent scans publish at different instants and
                // deliberately carry different format and snapshot identities.
                for key in [
                    "snapshot_id",
                    "schema_version",
                    "scanned_at",
                    "prev_scanned_at",
                    "age_seconds",
                ] {
                    fields.remove(key);
                }
                for value in fields.values_mut() {
                    normalize(value);
                }
            }
            Value::Array(values) => values.iter_mut().for_each(normalize),
            _ => {}
        }
    }
    let mut value = serde_json::to_value(value).unwrap();
    normalize(&mut value);
    value
}

fn parity<T: Serialize>(old: T, new: T) {
    assert_eq!(comparable(old), comparable(new));
}

fn check_listing(f: &Fx, reader: &Reader) {
    let old = Snapshot::open_current(&f.dir).unwrap();
    for sort in ["allocated", "apparent", "mtime"] {
        for limit in [1, 3, 1000] {
            let p = LsParams {
                path: Some(f.s("")),
                sort: Some(sort.into()),
                limit: Some(limit),
                ..Default::default()
            };
            parity(queries::ls(&old, &p).unwrap(), v2::ls(reader, &p).unwrap());
            check_top(f, reader, &old, sort, limit);
        }
    }
    for kind in [None, Some("any"), Some("file"), Some("dir")] {
        check_find(f, reader, &old, kind);
    }
    for (older, newer) in [
        (Some("2000-01-01"), None),
        (Some("2020-01-01"), None),
        (None, Some("2000-01-01")),
        (Some("2020-01-01"), Some("2000-01-01")),
    ] {
        for kind in [None, Some("dir"), Some("any")] {
            let p = FindParams {
                path: Some(f.s("")),
                older_than: older.map(str::to_owned),
                newer_than: newer.map(str::to_owned),
                kind: kind.map(str::to_owned),
                iname: Some("A*".into()),
                ..Default::default()
            };
            parity(
                queries::find(&old, &p).unwrap(),
                v2::find(reader, &p).unwrap(),
            );
        }
    }
}

fn check_top(f: &Fx, reader: &Reader, old: &Snapshot, sort: &str, limit: u32) {
    for kind in ["file", "dir"] {
        for min in [None, Some("4096"), Some("1M")] {
            for older in [None, Some("2000-01-01"), Some("2020-01-01")] {
                let p = TopParams {
                    path: Some(f.s("")),
                    kind: Some(kind.into()),
                    sort: Some(sort.into()),
                    limit: Some(limit),
                    min_size: min.map(str::to_owned),
                    older_than: older.map(str::to_owned),
                    ..Default::default()
                };
                parity(queries::top(old, &p).unwrap(), v2::top(reader, &p).unwrap());
            }
        }
    }
}

fn check_find(f: &Fx, reader: &Reader, old: &Snapshot, kind: Option<&str>) {
    for sort in ["allocated", "mtime"] {
        for limit in [1, 3, 1000] {
            for pattern in [None, Some("*"), Some("a*"), Some("?.txt")] {
                let p = FindParams {
                    path: Some(f.s("")),
                    kind: kind.map(str::to_owned),
                    sort: Some(sort.into()),
                    limit: Some(limit),
                    name: pattern.map(str::to_owned),
                    ..Default::default()
                };
                parity(
                    queries::find(old, &p).unwrap(),
                    v2::find(reader, &p).unwrap(),
                );
            }
        }
    }
}

#[test]
fn listings_filters_nested_totals_and_byte_names_match_v1() {
    let f = fx();
    old_file(&f, "a/z", 17);
    old_file(&f, "a-", 17);
    old_file(&f, "a/a.txt", 8193);
    old_file(&f, "b/alpha.txt", 17);
    old_file(&f, "A.txt", 17);
    fs::create_dir_all(f.p("empty/inside")).unwrap();
    fs::hard_link(f.p("a/z"), f.p("alias")).unwrap();
    std::os::unix::fs::symlink("a/z", f.p("symlink")).unwrap();
    let raw = f
        .root
        .join(std::ffi::OsString::from_vec(b"x\xff.txt".to_vec()));
    fs::write(&raw, b"bytes").unwrap();
    let (mut store, path) = store(&f);
    scan(&f, &mut store);
    let reader = Reader::open(&path).unwrap();
    check_listing(&f, &reader);
    let old = Snapshot::open_current(&f.dir).unwrap();
    let p = LsParams {
        path_raw: Some(hex_encode(raw.as_os_str().as_bytes())),
        ..Default::default()
    };
    parity(queries::ls(&old, &p).unwrap(), v2::ls(&reader, &p).unwrap());
    let response = v2::find(
        &reader,
        &FindParams {
            path: Some(f.s("")),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(response.entries.iter().any(|e| e.nested));
    assert!(response.total.count > response.total.counted);
}

#[test]
fn refresh_insertion_order_and_public_diff_match_v1() {
    let f = fx();
    old_file(&f, "z/z", 17);
    old_file(&f, "z/z-", 17);
    old_file(&f, "gone/a", 17);
    old_file(&f, "nest/a/z", 17);
    old_file(&f, "nest/a-/z", 17);
    let (mut store, path) = store(&f);
    scan(&f, &mut store);
    let stale = Reader::open(&path).unwrap();
    let old_id = stale.meta(false).unwrap().snapshot_id;
    fs::remove_dir_all(f.p("gone")).unwrap();
    old_file(&f, "z/a", 17);
    old_file(&f, "new/a", 8193);
    scan(&f, &mut store);
    old_file(&f, "z/0", 17);
    f.try_refresh("z").unwrap();
    store.refresh(&f.p("z"), 1).unwrap();
    let reader = Reader::open(&path).unwrap();
    check_listing(&f, &reader);
    let old_status = queries::status(&f.dir).unwrap();
    let new_status = v2::status(&reader, &f.dir).unwrap();
    assert_eq!(old_status.entries, new_status.entries);
    assert_eq!(old_status.generation, new_status.generation);
    assert_eq!(old_status.prev_generation, new_status.prev_generation);
    parity(old_status.prev, new_status.prev);
    parity(old_status.refreshed, new_status.refreshed);
    for depth in [1, 2, 3] {
        for kind in ["any", "file", "dir"] {
            for limit in [1, 2, 1000] {
                let p = DiffParams {
                    path: Some(f.s("")),
                    depth: Some(depth),
                    kind: Some(kind.into()),
                    limit: Some(limit),
                    ..Default::default()
                };
                parity(
                    queries::diff(&f.dir, &p).unwrap(),
                    v2::diff(&reader, &p).unwrap(),
                );
            }
        }
    }
    for rel in ["gone", "new"] {
        let p = DiffParams {
            path: Some(f.s(rel)),
            kind: Some("any".into()),
            ..Default::default()
        };
        parity(
            queries::diff(&f.dir, &p).unwrap(),
            v2::diff(&reader, &p).unwrap(),
        );
    }
    let p = LsParams {
        path: Some(f.s("")),
        snapshot: Some(old_id),
        ..Default::default()
    };
    assert_eq!(v2::ls(&reader, &p).unwrap_err().code, ErrorCode::Rotated);
    assert!(
        v2::ls(&stale, &p).is_ok(),
        "older reader retains its own identity"
    );
}

#[test]
fn errors_normalization_and_pass_contract_match_v1() {
    let f = fx();
    old_file(&f, "a/file", 17);
    let (mut store, path) = store(&f);
    scan(&f, &mut store);
    let reader = Reader::open(&path).unwrap();
    let old = Snapshot::open_current(&f.dir).unwrap();
    for rel in [
        format!("{}/a/../a//", f.s("")),
        f.s("missing/deep"),
        f.s("missing\0/deep"),
        "/unscanned-root".into(),
    ] {
        let p = LsParams {
            path: Some(rel),
            ..Default::default()
        };
        match (queries::ls(&old, &p), v2::ls(&reader, &p)) {
            (Ok(a), Ok(b)) => parity(a, b),
            (Err(a), Err(b)) => {
                assert_eq!(a.code, b.code);
                assert_eq!(a.fix, b.fix);
            }
            _ => panic!("different resolution outcomes"),
        }
    }
    for p in [
        FindParams {
            name: Some("a".into()),
            iname: Some("b".into()),
            ..Default::default()
        },
        FindParams {
            name: Some("\\".into()),
            ..Default::default()
        },
        FindParams {
            sort: Some("apparent".into()),
            ..Default::default()
        },
        FindParams {
            limit: Some(0),
            ..Default::default()
        },
        FindParams {
            min_size: Some("bad".into()),
            ..Default::default()
        },
    ] {
        assert_eq!(
            queries::find(&old, &p).unwrap_err().code,
            v2::find(&reader, &p).unwrap_err().code
        );
    }
    check_pass(&reader, &old, f.root.as_os_str().as_bytes());
    let missing_previous = v2::diff(
        &reader,
        &DiffParams {
            path: Some(f.s("")),
            ..Default::default()
        },
    )
    .unwrap_err();
    assert_eq!(missing_previous.code, ErrorCode::NoPrev);
}

fn check_pass(reader: &Reader, old: &Snapshot, path: &[u8]) {
    let scope = reader.get(path, false).unwrap().unwrap();
    let old_scope = queries::common::resolve(old, path).unwrap().0;
    let filter = queries::pass::Filter {
        kind: KindFilter::Any,
        min_size: 0,
        older: None,
        newer: None,
        name: None,
    };
    let order = queries::pass::Order {
        sort: SortKey::Allocated,
        limit: 1,
        list_nested: true,
    };
    let expected =
        queries::pass::run(old, (old_scope.id, old_scope.subtree_end), &filter, &order).unwrap();
    let actual = v2::pass::run(reader, &scope, &filter, &order).unwrap();
    assert_eq!(expected.total, actual.total);
    assert_eq!(actual.page.len(), 1);
    assert_eq!(expected.page[0].nested, actual.page[0].nested);
    assert_eq!(expected.page[0].key, actual.page[0].key);
}

#[test]
fn status_roots_metadata_and_first_run_match_v1() {
    let f = fx();
    old_file(&f, "z/file", 17);
    old_file(&f, "a/file", 17);
    let roots = vec![f.p("z"), f.p("a")];
    let (mut store, path) = store(&f);
    f.scan_roots(roots.clone(), 1, true).unwrap();
    store.scan_roots(&roots, true, 1).unwrap();
    let reader = Reader::open(&path).unwrap();
    let old = queries::status(&f.dir).unwrap();
    let new = v2::status(&reader, &f.dir).unwrap();
    assert_eq!(old.entries, new.entries);
    assert_eq!(
        old.roots
            .iter()
            .map(|r| (&r.path, r.items, r.allocated))
            .collect::<Vec<_>>(),
        new.roots
            .iter()
            .map(|r| (&r.path, r.items, r.allocated))
            .collect::<Vec<_>>()
    );
    assert_eq!(new.roots[0].path, f.s("z"));
    let mut old = comparable(old);
    let mut new = comparable(new);
    // Database publication itself changes df numbers between these independent
    // scans. Root identity, counts, capacity and state remain comparable.
    for value in [&mut old, &mut new] {
        for root in value["roots"].as_array_mut().unwrap() {
            root.as_object_mut().unwrap().remove("fs_used");
            root.as_object_mut().unwrap().remove("fs_avail");
        }
        for fs in value["fs"].as_array_mut().unwrap() {
            fs.as_object_mut().unwrap().remove("used");
            fs.as_object_mut().unwrap().remove("avail");
        }
    }
    assert_eq!(old, new);
    let empty = SnapshotDir::new(f.dir.path.with_extension("empty"));
    parity(
        queries::status::status_page(&empty).unwrap(),
        v2::status_page(None, &empty).unwrap(),
    );
}

#[test]
fn denied_previous_descendants_are_unknown_and_headers_global() {
    let f = fx();
    old_file(&f, "denied/child", 17);
    old_file(&f, "other/file", 17);
    let (mut store, path) = store(&f);
    scan(&f, &mut store);
    fs::remove_file(f.p("denied/child")).unwrap();
    scan(&f, &mut store);
    let c = rusqlite::Connection::open(f.dir.current()).unwrap();
    let id: i64 = c
        .query_row(
            "SELECT id FROM entries WHERE name=?1",
            [b"denied".as_slice()],
            |r| r.get(0),
        )
        .unwrap();
    c.execute("UPDATE entries SET flags=flags|1 WHERE id=?1", [id])
        .unwrap();
    c.execute("INSERT INTO denied(entry_id) VALUES(?1)", [id])
        .unwrap();
    drop(c);
    let c = rusqlite::Connection::open(&path).unwrap();
    c.execute(
        "UPDATE live_rows SET flags=flags|1 WHERE id IN (SELECT id FROM identities WHERE path=?1)",
        [f.p("denied").as_os_str().as_bytes()],
    )
    .unwrap();
    c.execute(
        "UPDATE version_rows SET flags=flags|1 WHERE id IN (SELECT id FROM identities WHERE path=?1) AND dead IS NULL",
        [f.p("denied").as_os_str().as_bytes()],
    )
    .unwrap();
    drop(c);
    let reader = Reader::open(&path).unwrap();
    let p = DiffParams {
        path: Some(f.s("")),
        depth: Some(2),
        kind: Some("file".into()),
        ..Default::default()
    };
    let actual = v2::diff(&reader, &p).unwrap();
    assert!(!actual.entries.iter().any(|e| e.name == "child"));
    parity(queries::diff(&f.dir, &p).unwrap(), actual);
    let p = LsParams {
        path: Some(f.s("other")),
        ..Default::default()
    };
    let response = v2::ls(&reader, &p).unwrap();
    assert!(response.header.incomplete);
    assert_eq!(response.header.unknown_subtrees, vec![f.s("denied")]);
    parity(
        queries::ls(&Snapshot::open_current(&f.dir).unwrap(), &p).unwrap(),
        response,
    );
}

#[test]
fn mount_placeholder_and_nested_root_with_identical_paths_stay_distinct() {
    let f = fx();
    fs::create_dir(f.p("mount")).unwrap();
    let (mut store, path) = store(&f);
    scan(&f, &mut store);
    let mount_path = f.p("mount");
    let raw = mount_path.as_os_str().as_bytes();
    // Model a cross-device boundary without requiring privileged mounts. The
    // separate root at the same path has its own entry and different totals.
    let c = rusqlite::Connection::open(f.dir.current()).unwrap();
    let id: i64 = c
        .query_row(
            "SELECT id FROM entries WHERE name=?1",
            [b"mount".as_slice()],
            |r| r.get(0),
        )
        .unwrap();
    c.execute("UPDATE entries SET kind=4 WHERE id=?1", [id])
        .unwrap();
    c.execute(
        "INSERT INTO mounts VALUES(?1,?2,'testfs',123,456)",
        rusqlite::params![id, raw],
    )
    .unwrap();
    c.execute("INSERT INTO entries SELECT ?1,NULL,?2,path_hash,1,0,999,999,mtime,nlink,0,0,scanned_at,?1,min_scanned_at,newest_mtime FROM entries WHERE id=?3", rusqlite::params![id+10000, raw, id]).unwrap();
    c.execute(
        "INSERT INTO roots VALUES(?1,?2,'testfs',123,456,333,NULL)",
        rusqlite::params![id + 10000, raw],
    )
    .unwrap();
    drop(c);
    let c = rusqlite::Connection::open(&path).unwrap();
    let id: i64 = c
        .query_row("SELECT id FROM live WHERE path=?1", [raw], |r| r.get(0))
        .unwrap();
    c.execute("UPDATE version_rows SET kind=4 WHERE id=?1", [id])
        .unwrap();
    c.execute(
        "INSERT INTO mount_versions VALUES(?1,?2,'testfs',123,456,1,NULL)",
        rusqlite::params![id, raw],
    )
    .unwrap();
    c.execute("INSERT INTO identities(id,scope,path,sort_key) SELECT ?1,path,path,sort_key FROM identities WHERE id=?2", rusqlite::params![id+10000, id]).unwrap();
    c.execute("INSERT INTO version_rows SELECT ?1,NULL,1,0,own_allocated,own_apparent,own_errors,mtime,scanned_at,999,999,0,0,min_scanned_at,newest_mtime,nlink,dev,ino,born,dead,basename FROM version_rows WHERE id=?2", rusqlite::params![id+10000, id]).unwrap();
    c.execute("INSERT INTO root_versions SELECT ?1,?2,dev,'testfs',456,333,333,123,inodes_total,inodes_free,1,born,dead FROM root_versions", rusqlite::params![id+10000, raw]).unwrap();
    drop(c);
    let reader = Reader::open(&path).unwrap();
    let old = Snapshot::open_current(&f.dir).unwrap();
    for rel in ["", "mount"] {
        let p = LsParams {
            path: Some(f.s(rel)),
            ..Default::default()
        };
        parity(queries::ls(&old, &p).unwrap(), v2::ls(&reader, &p).unwrap());
    }
    let listing = v2::ls(
        &reader,
        &LsParams {
            path: Some(f.s("")),
            ..Default::default()
        },
    )
    .unwrap();
    let mount = &listing.entries[0];
    assert_eq!(mount.kind, freespace_core::scan::Kind::Mount);
    assert_eq!(mount.mount.as_ref().unwrap().root, Some(f.s("mount")));
    let root = v2::ls(
        &reader,
        &LsParams {
            path: Some(f.s("mount")),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(root.entry.allocated, 999);
    assert_eq!(root.entry.name, f.s("mount"));
}

#[test]
fn public_queries_survive_blocked_maintenance_then_gc() {
    let f = fx();
    old_file(&f, "local/old", 17);
    let (mut store, path) = store(&f);
    scan(&f, &mut store);
    old_file(&f, "local/new", 17);
    scan(&f, &mut store);
    let pinned = Reader::open(&path).unwrap();
    let ls = LsParams {
        path: Some(f.s("")),
        ..Default::default()
    };
    let top = TopParams {
        path: Some(f.s("")),
        ..Default::default()
    };
    let find = FindParams {
        path: Some(f.s("")),
        ..Default::default()
    };
    let diff = DiffParams {
        path: Some(f.s("")),
        kind: Some("any".into()),
        ..Default::default()
    };
    let expected = (
        v2::ls(&pinned, &ls).unwrap(),
        v2::top(&pinned, &top).unwrap(),
        v2::find(&pinned, &find).unwrap(),
        v2::diff(&pinned, &diff).unwrap(),
    );
    for size in [8193, 16385, 32769] {
        old_file(&f, "local/new", size);
        f.try_refresh("local").unwrap();
        store.refresh(&f.p("local"), 1).unwrap();
    }
    let report = store.maintain(1000, true).unwrap();
    assert!(report.readers_blocking);
    assert_eq!(report.deleted, 0);
    parity(expected.0, v2::ls(&pinned, &ls).unwrap());
    parity(expected.1, v2::top(&pinned, &top).unwrap());
    parity(expected.2, v2::find(&pinned, &find).unwrap());
    parity(expected.3, v2::diff(&pinned, &diff).unwrap());
    assert_eq!(v2::status(&pinned, &f.dir).unwrap().generation, Some(2));
    drop(pinned);
    let mut deleted = 0;
    for _ in 0..8 {
        let report = store.maintain(1000, true).unwrap();
        assert!(!report.readers_blocking);
        deleted += report.deleted;
    }
    assert!(deleted > 0, "retired intermediate versions are reclaimed");
    let current = Reader::open(&path).unwrap();
    let old = Snapshot::open_current(&f.dir).unwrap();
    parity(
        queries::ls(&old, &ls).unwrap(),
        v2::ls(&current, &ls).unwrap(),
    );
    parity(
        queries::top(&old, &top).unwrap(),
        v2::top(&current, &top).unwrap(),
    );
    parity(
        queries::find(&old, &find).unwrap(),
        v2::find(&current, &find).unwrap(),
    );
    parity(
        queries::diff(&f.dir, &diff).unwrap(),
        v2::diff(&current, &diff).unwrap(),
    );
    assert_eq!(v2::status(&current, &f.dir).unwrap().generation, Some(5));
}

#[test]
fn global_denied_paths_follow_configured_root_order() {
    let f = fx();
    old_file(&f, "z/denied/file", 17);
    old_file(&f, "a/denied/file", 17);
    let roots = vec![f.p("z"), f.p("a")];
    let (mut store, path) = store(&f);
    f.scan_roots(roots.clone(), 1, true).unwrap();
    store.scan_roots(&roots, true, 1).unwrap();
    let c = rusqlite::Connection::open(f.dir.current()).unwrap();
    c.execute("UPDATE entries SET flags=flags|1 WHERE kind=1", [])
        .unwrap();
    c.execute("INSERT INTO denied SELECT id FROM entries WHERE kind=1", [])
        .unwrap();
    drop(c);
    let c = rusqlite::Connection::open(&path).unwrap();
    c.execute("UPDATE version_rows SET flags=flags|1 WHERE kind=1", [])
        .unwrap();
    drop(c);
    let reader = Reader::open(&path).unwrap();
    let old = Snapshot::open_current(&f.dir).unwrap();
    let p = LsParams {
        path: Some(f.s("a")),
        ..Default::default()
    };
    let response = v2::ls(&reader, &p).unwrap();
    assert_eq!(response.header.unknown_subtrees, vec![f.s("z"), f.s("a")]);
    parity(queries::ls(&old, &p).unwrap(), response);
}

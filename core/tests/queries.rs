//! Query, diff and refresh behaviour on real scans of small fixture trees
//! (T5, T15, T19, T33). Each test builds its own tree in a temp dir, scans it
//! into a temp snapshot dir and asserts on the answers the CLI and API print.

use std::fs::{self, File};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::time::{Duration, UNIX_EPOCH};

use freespace_core::ErrorCode;
use freespace_core::job::ScanReport;
use freespace_core::platform::now_secs;
use freespace_core::queries::pass::{self, Filter, Order};
use freespace_core::queries::{self, diff, params::*, types::*};
use freespace_core::refresh::{self, RefreshReport, RefreshRequest};
use freespace_core::scan::Kind;
use freespace_core::snapshot::reader::Snapshot;

const DAY: i64 = 86_400;

mod common;
use common::{Fx, fx};

impl Fx {
    fn mkdir(&self, rel: &str) {
        fs::create_dir_all(self.p(rel)).unwrap();
    }

    /// A file of `size` real bytes (not sparse) with the given mtime.
    fn file(&self, rel: &str, size: usize, mtime: i64) {
        self.write_file(rel, size);
        set_mtime(&self.p(rel), mtime);
    }

    fn scan(&self, keep_previous: bool) -> ScanReport {
        self.scan_roots(vec![self.root.clone()], 4, keep_previous)
            .unwrap()
    }

    fn refresh(&self, rel: &str) -> RefreshReport {
        self.try_refresh(rel).unwrap()
    }

    fn snap(&self) -> Snapshot {
        Snapshot::open_current(&self.dir).unwrap()
    }

    fn ls(&self, rel: &str) -> LsResponse {
        queries::ls(
            &self.snap(),
            &LsParams {
                path: Some(self.s(rel)),
                limit: Some(1000),
                ..Default::default()
            },
        )
        .unwrap()
    }

    fn entry(&self, rel: &str) -> EntryOut {
        self.ls(rel).entry
    }

    fn top(&self, p: TopParams) -> TopResponse {
        queries::top(
            &self.snap(),
            &TopParams {
                path: Some(self.s("")),
                ..p
            },
        )
        .unwrap()
    }

    fn find(&self, p: FindParams) -> FindResponse {
        queries::find(
            &self.snap(),
            &FindParams {
                path: Some(self.s("")),
                ..p
            },
        )
        .unwrap()
    }
}

fn set_mtime(p: &Path, t: i64) {
    let f = File::open(p).unwrap();
    let st = if t >= 0 {
        UNIX_EPOCH + Duration::from_secs(t as u64)
    } else {
        UNIX_EPOCH - Duration::from_secs(t.unsigned_abs())
    };
    f.set_modified(st).unwrap();
}

fn names(v: &[EntryOut]) -> Vec<String> {
    v.iter().map(|e| e.name.clone()).collect()
}

fn sorted(mut v: Vec<String>) -> Vec<String> {
    v.sort();
    v
}

fn is_root() -> bool {
    unsafe { libc::geteuid() == 0 }
}

#[test]
fn ls_children_biggest_first_with_totals() {
    let f = fx();
    let now = now_secs();
    f.file("big", 200 << 10, now - 10 * DAY);
    f.file("small", 10 << 10, now - 30 * DAY);
    f.file("d/x", 100 << 10, now - 20 * DAY);
    f.scan(false);

    let r = f.ls("");
    assert_eq!(names(&r.entries), ["big", "d", "small"]);
    assert_eq!(r.total_count, 3);
    assert!(!r.truncated);
    assert_eq!(r.entry.items, 4);
    let own_dir = r.entry.allocated - r.entries.iter().map(|e| e.allocated).sum::<u64>();
    assert!(own_dir <= 64 << 10, "root's own blocks only: {own_dir}");
    assert!(!r.header.incomplete);
    assert_eq!(r.header.generation, 1);

    let by_mtime = queries::ls(
        &f.snap(),
        &LsParams {
            path: Some(f.s("")),
            sort: Some("mtime".into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        by_mtime.entries[0].name, "small",
        "mtime sort is oldest first"
    );

    let two = queries::ls(
        &f.snap(),
        &LsParams {
            path: Some(f.s("")),
            limit: Some(2),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(two.entries.len(), 2);
    assert!(two.truncated);

    // relative ".." segments are normalized; unknown paths name the nearest ancestor
    assert_eq!(f.ls("d/../d").entry.name, "d");
    let e = queries::ls(
        &f.snap(),
        &LsParams {
            path: Some(f.s("d/nope/deeper")),
            ..Default::default()
        },
    )
    .unwrap_err();
    assert_eq!(e.code, ErrorCode::PathNotFound);
    assert!(
        e.fix.as_deref().unwrap_or("").contains(&f.s("d")),
        "{:?}",
        e.fix
    );
}

/// The 179/180/181-day boundary for files (the "line 649" fixture): must stay
/// unchanged with the explicit `--kind file` and the default kind.
#[test]
fn top_files_older_than_boundary() {
    let f = fx();
    let now = now_secs();
    f.file("f179", 4096, now - 179 * DAY);
    f.file("f180_newer", 4096, now - 180 * DAY + 3600);
    f.file("f180_older", 4096, now - 180 * DAY - 3600);
    f.file("f181", 4096, now - 181 * DAY);
    f.file("today", 4096, now);
    f.mkdir("emptydir");
    f.scan(false);

    for kind in [None, Some("file".to_string())] {
        let r = f.top(TopParams {
            kind: kind.clone(),
            older_than: Some("180d".into()),
            ..Default::default()
        });
        assert_eq!(r.kind, "file");
        assert_eq!(
            sorted(names(&r.entries)),
            ["f180_older", "f181"],
            "kind {kind:?}"
        );
        assert!(r.entries.iter().all(|e| e.kind == Kind::File));
        assert_eq!(r.total.count, 2);
    }
}

#[test]
fn top_dirs_untouched_are_topmost_and_totalled_once() {
    let f = fx();
    let now = now_secs();
    f.file("old/a", 64 << 10, now - 400 * DAY);
    f.file("old/sub/b", 32 << 10, now - 400 * DAY);
    f.file("fresh/y", 4096, now - DAY);
    set_mtime(&f.p("fresh"), now - 6 * 365 * DAY); // dir mtime is ignored
    f.file("b179/f", 4096, now - 179 * DAY);
    f.file("b181/f", 8192, now - 181 * DAY);
    f.mkdir("empty");
    f.mkdir("nofiles/sub");
    f.scan(false);

    let all = f.top(TopParams {
        kind: Some("dir".into()),
        older_than: Some("180d".into()),
        ..Default::default()
    });
    assert_eq!(all.kind, "dir");
    // empty and dirs-only subtrees have newest_mtime 0 and pass (D1 = B)
    assert_eq!(
        sorted(names(&all.entries)),
        ["b181", "empty", "nofiles", "old"]
    );
    assert_eq!(all.total.count, 4);
    let sum: u64 = all.entries.iter().map(|e| e.allocated).sum();
    assert_eq!(all.total.allocated, sum);
    assert_eq!(all.entries[0].name, "old", "biggest first");
    let old = &all.entries[0];
    assert_eq!(old.newest_mtime, now - 400 * DAY);
    for e in &all.entries {
        assert!(e.newest_mtime <= e.scanned_at);
    }

    let one = f.top(TopParams {
        kind: Some("dir".into()),
        older_than: Some("180d".into()),
        limit: Some(1),
        ..Default::default()
    });
    assert_eq!(one.entries.len(), 1);
    assert!(one.truncated);
    assert_eq!(
        one.total, all.total,
        "totals cover every topmost match, not the page"
    );

    let by_mtime = f.top(TopParams {
        kind: Some("dir".into()),
        older_than: Some("180d".into()),
        sort: Some("mtime".into()),
        ..Default::default()
    });
    assert_eq!(
        sorted(names(&by_mtime.entries)),
        ["b181", "empty", "nofiles", "old"]
    );
    assert_eq!(
        by_mtime.entries.last().map(|e| e.name.as_str()),
        Some("b181")
    );
}

/// `find` compares `newest_mtime` on directory rows (D2) and implies files when
/// only a date filter is given.
#[test]
fn find_dirs_use_newest_file_inside() {
    let f = fx();
    let now = now_secs();
    f.file("y2019/new.txt", 4096, now - DAY);
    set_mtime(&f.p("y2019"), now - 7 * 365 * DAY);
    f.file("old2y/old.txt", 4096, now - 2 * 365 * DAY);
    set_mtime(&f.p("old2y"), now - DAY);
    f.scan(false);

    let old = f.find(FindParams {
        kind: Some("dir".into()),
        older_than: Some("1y".into()),
        ..Default::default()
    });
    let got: Vec<&str> = old.entries.iter().map(|e| e.entry.name.as_str()).collect();
    assert_eq!(got, ["old2y"]);

    let recent = f.find(FindParams {
        kind: Some("dir".into()),
        newer_than: Some("7d".into()),
        ..Default::default()
    });
    let y = recent
        .entries
        .iter()
        .find(|e| e.entry.name == "y2019")
        .expect("dir whose only change is a nested file");
    assert!(!y.nested);
    assert!(
        !recent.entries.iter().any(|e| e.entry.path == f.s("")),
        "find searches within the path, not the path itself"
    );
    assert!(!recent.entries.iter().any(|e| e.entry.name == "old2y"));

    let files = f.find(FindParams {
        older_than: Some("1y".into()),
        ..Default::default()
    });
    assert_eq!(files.kind, "file");
    let got: Vec<&str> = files
        .entries
        .iter()
        .map(|e| e.entry.name.as_str())
        .collect();
    assert_eq!(got, ["old.txt"]);
}

#[test]
fn find_nested_matches_counted_once() {
    let f = fx();
    let now = now_secs();
    f.file("p1/node_modules/a", 50 << 10, now);
    f.file("p1/node_modules/x/node_modules/b", 20 << 10, now);
    f.file("p2/node_modules/c", 30 << 10, now);
    f.file("p2/src/main.rs", 4096, now);
    f.scan(false);

    let r = f.find(FindParams {
        name: Some("node_modules".into()),
        ..Default::default()
    });
    assert_eq!(r.total.count, 3);
    assert_eq!(r.total.counted, 2);
    let top_level: u64 = r
        .entries
        .iter()
        .filter(|e| !e.nested)
        .map(|e| e.entry.allocated)
        .sum();
    assert_eq!(r.total.allocated, top_level);
    let inner = r
        .entries
        .iter()
        .find(|e| e.entry.path.ends_with("x/node_modules"))
        .unwrap();
    assert!(inner.nested);

    let i = f.find(FindParams {
        iname: Some("NODE_*".into()),
        limit: Some(1),
        ..Default::default()
    });
    assert_eq!(i.total, r.total);
    assert_eq!(i.entries.len(), 1);
    assert!(i.truncated);
}

fn populate_split_pass_directory(f: &Fx, p: usize, depth: usize, now: i64) {
    let dir = format!("p{p}{}", "/node_modules/m".repeat(depth));
    for i in 0..4usize {
        // all of p0 and every deepest folder untouched, the rest mixed
        let aged = p == 0 || depth == 3 || (p + i).is_multiple_of(2);
        let t = if aged { now - 400 * DAY } else { now };
        f.file(&format!("{dir}/f{i}.js"), (i + 1) * depth * 4096, t);
    }
}

/// Wide subtrees are read as parallel id ranges, a selective --min-size
/// through the size index. Every split and the index answer like one pass,
/// `nested` included, when a matched directory starts in an earlier range.
#[test]
fn split_passes_answer_like_one_pass() {
    let f = fx();
    let now = now_secs();
    for p in 0..3usize {
        for depth in 1..=3usize {
            populate_split_pass_directory(&f, p, depth, now);
        }
    }
    f.scan(false);
    let snap = Snapshot::open_current(&f.dir).unwrap();
    let cases = || {
        let filter = |kind, min_size, older, name: Option<&str>| Filter {
            kind,
            min_size,
            older,
            newer: None,
            name: name.map(|n| (n.to_owned(), false)),
        };
        let order = |sort, limit, list_nested| Order {
            sort,
            limit,
            list_nested,
        };
        let old = Some(now - 180 * DAY);
        [
            (
                filter(KindFilter::Dir, 0, None, Some("node_modules")),
                order(SortKey::Allocated, 5, true),
            ),
            (
                filter(KindFilter::Any, 0, None, None),
                order(SortKey::Mtime, 3, true),
            ),
            (
                filter(KindFilter::Dir, 0, old, None),
                order(SortKey::Allocated, 4, false),
            ),
            (
                filter(KindFilter::File, 8192, None, None),
                order(SortKey::Apparent, 2, false),
            ),
            (
                filter(KindFilter::Any, 0, old, Some("*")),
                order(SortKey::Allocated, 50, true),
            ),
        ]
    };
    for rel in ["", "p1"] {
        let (scope, _) = queries::common::resolve(&snap, f.s(rel).as_bytes()).unwrap();
        let scope = (scope.id, scope.subtree_end);
        for (i, (filter, order)) in cases().iter().enumerate() {
            let one = pass::run_in(&snap, scope, filter, order, 1).unwrap();
            // a small fixture: any --min-size reads through entries_alloc
            assert_eq!(pass::run(&snap, scope, filter, order).unwrap(), one);
            assert!(
                one.total.count > 0,
                "case {i} under {rel:?} matches nothing"
            );
            if i == 0 {
                assert!(one.total.count > one.total.counted, "no nested matches");
            }
            for n in 2..=12 {
                let split = pass::run_in(&snap, scope, filter, order, n).unwrap();
                assert_eq!(split, one, "case {i} under {rel:?} in {n} ranges");
            }
        }
    }
}

#[test]
fn hardlinks_counted_once_at_the_smallest_path() {
    let f = fx();
    let now = now_secs();
    f.file("a/x", 100 << 10, now);
    f.mkdir("b");
    fs::hard_link(f.p("a/x"), f.p("b/y")).unwrap();
    f.scan(false);

    let a = f.ls("a");
    let x = &a.entries[0];
    assert!(x.allocated >= 100 << 10);
    assert!(!x.hardlink_alias);
    assert_eq!(x.nlink, 2);
    let b = f.ls("b");
    let y = &b.entries[0];
    assert!(y.hardlink_alias);
    assert_eq!(y.allocated, 0);
    assert_eq!(y.hardlink_owner.as_deref(), Some(f.s("a/x").as_str()));
    assert!(b.entry.allocated < 100 << 10);
}

/// T19 + T33 F2: refresh keeps additive aggregates exact and re-folds
/// `newest_mtime` up the ancestor chain, lowering it when the newest file goes.
#[test]
fn refresh_refolds_newest_mtime_on_ancestors() {
    let f = fx();
    let now = now_secs();
    f.file("rf/a/f", 8192, now - 2 * 365 * DAY);
    f.file("rf/b/g", 8192, now - DAY);
    f.file("sib/h", 8192, now - 3 * 365 * DAY);
    f.scan(false);
    assert_eq!(f.entry("rf").newest_mtime, now - DAY);
    assert_eq!(f.entry("").newest_mtime, now - DAY);
    let root0 = f.entry("");
    let sib0 = f.entry("sib");

    // nothing changed on disk: every aggregate is identical
    let rep = f.refresh("rf");
    assert_eq!(rep.generation, 2);
    let root1 = f.entry("");
    assert_eq!(
        (
            root1.allocated,
            root1.apparent,
            root1.items,
            root1.errors,
            root1.newest_mtime
        ),
        (
            root0.allocated,
            root0.apparent,
            root0.items,
            root0.errors,
            root0.newest_mtime
        )
    );

    // the newest file goes: b, rf and the root are lowered
    let g = f.ls("rf/b").entries[0].allocated;
    fs::remove_file(f.p("rf/b/g")).unwrap();
    f.refresh("rf/b");
    assert_eq!(f.entry("rf/b").newest_mtime, 0);
    assert_eq!(f.entry("rf").newest_mtime, now - 2 * 365 * DAY);
    let root2 = f.entry("");
    assert_eq!(root2.newest_mtime, now - 2 * 365 * DAY);
    assert_eq!(root2.items, root0.items - 1);
    assert_eq!(root2.allocated, root0.allocated - g);
    assert_eq!(f.entry("sib"), sib0, "untouched sibling keeps its row");

    // a newer file appears: ancestors are raised, clamped to the refresh time
    f.file("rf/a/new", 4096, now);
    let rep = f.refresh("rf/a");
    let rf = f.entry("rf");
    assert!(rf.newest_mtime >= now - 1 && rf.newest_mtime <= rf.scanned_at.max(now));
    assert_eq!(f.entry("").newest_mtime, rf.newest_mtime);
    assert_eq!(rep.subtree_entries, 3);

    // refreshes are listed and the full scan time is kept
    let snap = f.snap();
    assert_eq!(snap.meta.generation, 4);
    let st = queries::status(&f.dir).unwrap();
    assert_eq!(st.refreshed.len(), 3);
    assert!(st.refreshed.iter().any(|r| r.path == f.s("rf/a")));
}

/// A refresh stamps the subtree it walks again with its own time, and only that
/// subtree, and the age a query reports is the oldest stamp under its path.
/// scripts/rust-diff.sh cannot compare these stamps (a scan stamps each folder
/// as it walks it, so two runs differ), so a refresh that stamps the wrong rows
/// shows here.
#[test]
fn refresh_stamps_only_the_subtree_it_walks() {
    let f = fx();
    let now = now_secs();
    f.file("rf/a/f", 8192, now - DAY);
    f.file("sib/h", 8192, now - DAY);
    f.scan(false);
    // the refresh starts in a later second than every stamp of the scan
    let scanned = now_secs();
    while now_secs() <= scanned {
        // gstack-timing-ok: waits for the clock's next second; only the order of the stamps is asserted
        std::thread::sleep(Duration::from_millis(20));
    }
    f.refresh("rf");
    // a folder's own stamp and its children's
    let stamps = |rel: &str| {
        let ls = f.ls(rel);
        let mut s = vec![ls.entry.scanned_at];
        s.extend(ls.entries.iter().map(|e| e.scanned_at));
        s
    };
    for rel in ["rf", "rf/a"] {
        assert!(
            stamps(rel).iter().all(|&t| t > scanned),
            "{rel}: stamped by the refresh"
        );
    }
    assert!(stamps("")[0] <= scanned, "the root keeps the scan's stamp");
    assert!(
        stamps("sib").iter().all(|&t| t <= scanned),
        "sib keeps the scan's stamps"
    );
    let before = now_secs();
    let (root, rf) = (f.ls("").header.age_seconds, f.ls("rf").header.age_seconds);
    let after = now_secs();
    assert!(
        root >= before - scanned,
        "the root's age is the scan's: {root}"
    );
    assert!(rf < after - scanned, "rf's age is the refresh's: {rf}");
}

#[test]
fn refresh_keeps_hardlinks_counted_once() {
    let f = fx();
    let now = now_secs();
    f.file("in/y", 100 << 10, now);
    f.mkdir("out");
    f.mkdir("other");
    fs::hard_link(f.p("in/y"), f.p("out/x")).unwrap();
    f.scan(false);
    let total = f.entry("").allocated;
    assert!(!f.ls("in").entries[0].hardlink_alias, "in/y < out/x");

    f.refresh("in");
    assert_eq!(f.entry("").allocated, total);
    assert!(!f.ls("in").entries[0].hardlink_alias);
    f.refresh("out");
    assert_eq!(f.entry("").allocated, total);
    assert!(f.ls("out").entries[0].hardlink_alias);

    // a new link inside the refreshed subtree whose other link is outside it and
    // unknown to the snapshot stays at 0 until the next full scan (OV-11)
    f.file("out/w", 50 << 10, now);
    fs::hard_link(f.p("out/w"), f.p("in/z")).unwrap();
    f.refresh("in");
    let z = f
        .ls("in")
        .entries
        .into_iter()
        .find(|e| e.name == "z")
        .unwrap();
    assert_eq!(z.allocated, 0);
    assert_eq!(
        f.entry("").allocated,
        total,
        "w is not in the snapshot yet, z adds nothing"
    );

    // both links new and inside: counted normally
    f.file("in/n1", 30 << 10, now);
    fs::hard_link(f.p("in/n1"), f.p("in/n2")).unwrap();
    f.refresh("in");
    let ins = f.ls("in").entries;
    let n1 = ins.iter().find(|e| e.name == "n1").unwrap();
    let n2 = ins.iter().find(|e| e.name == "n2").unwrap();
    assert!(n1.allocated >= 30 << 10);
    assert_eq!(n2.allocated, 0);
    assert!(n2.hardlink_alias);

    // the zeroed z stays at 0 through later refreshes, inside and elsewhere
    let z_of = || {
        f.ls("in")
            .entries
            .into_iter()
            .find(|e| e.name == "z")
            .unwrap()
    };
    let before = f.entry("").allocated;
    f.refresh("in");
    assert_eq!(f.entry("").allocated, before, "second refresh of in");
    f.refresh("other");
    assert_eq!(f.entry("").allocated, before, "unrelated refresh");
    assert_eq!(z_of().allocated, 0);

    // refreshing the other side observes both links: counted once, at the smaller path
    f.refresh("out");
    let z = z_of();
    assert!(z.allocated >= 50 << 10 && !z.hardlink_alias);
    let w = f
        .ls("out")
        .entries
        .into_iter()
        .find(|e| e.name == "w")
        .unwrap();
    assert_eq!(w.allocated, 0);
    assert!(w.hardlink_alias);
    assert_eq!(f.entry("").allocated, before + z.allocated);
}

#[test]
fn huge_min_size_matches_nothing() {
    let f = fx();
    f.file("a", 10 << 10, now_secs());
    f.scan(false);
    let r = f.top(TopParams {
        min_size: Some("8388608T".into()),
        ..Default::default()
    });
    assert_eq!(r.total_count, 0);
    let r = queries::find(
        &f.snap(),
        &FindParams {
            path: Some(f.s("")),
            min_size: Some("8388608T".into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(r.total_count, 0);
}

#[test]
fn diff_ties_are_ordered_by_path() {
    let f = fx();
    for d in ["p1", "p2", "p3", "p4"] {
        f.file(&format!("{d}/x/f"), 10 << 10, now_secs());
    }
    f.scan(true);
    for d in ["p1", "p2", "p3", "p4"] {
        fs::remove_dir_all(f.p(&format!("{d}/x"))).unwrap();
    }
    f.scan(true);
    for _ in 0..4 {
        let r = queries::diff(
            &f.dir,
            &DiffParams {
                path: Some(f.s("")),
                depth: Some(2),
                kind: Some("dir".into()),
                limit: Some(1),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(
            r.entries[0].path.ends_with("/p1/x"),
            "{}",
            r.entries[0].path
        );
    }
}

#[test]
fn diff_pairs_generations_and_finds_growth() {
    let f = fx();
    let now = now_secs();
    f.file("grow/sub/a", 10 << 10, now);
    f.file("stay/b", 10 << 10, now);
    f.file("gone/c", 10 << 10, now);
    f.scan(true);
    let e = queries::diff(&f.dir, &DiffParams::default()).unwrap_err();
    assert_eq!(e.code, ErrorCode::NoPrev);
    assert_eq!(e.exit(), 3);

    f.file("grow/sub/big", 500 << 10, now);
    fs::remove_dir_all(f.p("gone")).unwrap();
    f.file("newdir/d", 20 << 10, now);
    let rep = f.scan(true);
    assert_eq!(rep.prev_generation, Some(1));

    let d = queries::diff(
        &f.dir,
        &DiffParams {
            path: Some(f.s("")),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(!d.partial);
    assert!(!d.roots_changed);
    assert_eq!(d.entries[0].name, "grow");
    assert!(d.entries[0].delta >= 500 << 10);
    let gone = d.entries.iter().find(|e| e.name == "gone").unwrap();
    assert_eq!(gone.status, DiffStatus::Gone);
    assert!(gone.delta < 0);
    let new = d.entries.iter().find(|e| e.name == "newdir").unwrap();
    assert_eq!(new.status, DiffStatus::New);
    let total: i64 = d.entries.iter().map(|e| e.delta).sum();
    assert!(d.delta >= total - (64 << 10) && d.delta <= total + (64 << 10));

    // status sentence clause: the growth sits in grow/sub
    let (cur, prev) = diff::open_pair(&f.dir).unwrap();
    let (path, delta) = diff::top_growth(&cur, &prev, f.s("").as_bytes())
        .unwrap()
        .unwrap();
    assert_eq!(path, f.s("grow/sub"));
    assert!(delta >= 500 << 10);
    drop((cur, prev));

    let st = queries::status(&f.dir).unwrap();
    assert_eq!(st.generation, Some(2));
    assert_eq!(st.prev.as_ref().map(|p| p.generation), Some(1));
    assert_eq!(st.roots[0].path, f.s(""));

    // refresh after prev: diff says partial and names the subtree
    f.refresh("stay");
    let d = queries::diff(
        &f.dir,
        &DiffParams {
            path: Some(f.s("")),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(d.partial);
    assert_eq!(d.refreshed[0].path, f.s("stay"));
}

#[test]
fn status_survives_a_corrupt_state_file() {
    let f = fx();
    f.file("a", 4096, now_secs());
    f.scan(false);
    fs::write(f.dir.state(), b"{not json").unwrap();
    let st = queries::status(&f.dir).unwrap();
    assert!(st.state_error.is_some());
    assert_eq!(st.generation, Some(1));
    assert!(st.last_error.is_none());
    assert_eq!(st.entries, Some(2));
}

#[test]
fn error_codes_for_missing_snapshot_prev_and_bad_input() {
    let f = fx();
    let Err(e) = Snapshot::open_current(&f.dir) else {
        panic!("no snapshot expected")
    };
    assert_eq!((e.code, e.exit()), (ErrorCode::NoSnapshot, 8));
    assert!(queries::status(&f.dir).is_err());

    f.file("a", 4096, now_secs());
    f.scan(false);
    let e = queries::diff(&f.dir, &DiffParams::default()).unwrap_err();
    assert_eq!((e.code, e.exit()), (ErrorCode::NoPrev, 3));

    let snap = f.snap();
    let bad = |p: LsParams| queries::ls(&snap, &p).unwrap_err();
    let e = bad(LsParams {
        sort: Some("size".into()),
        ..Default::default()
    });
    assert_eq!((e.code, e.exit()), (ErrorCode::Usage, 2));
    let e = bad(LsParams {
        limit: Some(0),
        ..Default::default()
    });
    assert_eq!(e.code, ErrorCode::Usage);
    let e = bad(LsParams {
        snapshot: Some("0000".into()),
        ..Default::default()
    });
    assert_eq!((e.code, e.exit()), (ErrorCode::Rotated, 4));
    let e = queries::top(
        &snap,
        &TopParams {
            min_size: Some("1.5G".into()),
            ..Default::default()
        },
    )
    .unwrap_err();
    assert_eq!(e.code, ErrorCode::Usage);
    let e = queries::find(
        &snap,
        &FindParams {
            kind: Some("folder".into()),
            ..Default::default()
        },
    )
    .unwrap_err();
    assert_eq!(e.code, ErrorCode::Usage);
}

/// D3/D4: a future-dated file pins its folder to the scan time, never past it;
/// the raw mtime is still reported on the file row.
#[test]
fn future_files_are_clamped_to_scan_time() {
    let f = fx();
    let now = now_secs();
    let future = now + 10 * 365 * DAY;
    f.file("d/old", 4096, now - 3 * 365 * DAY);
    f.file("d/fut", 4096, future);
    f.scan(false);
    let d = f.ls("d");
    let fut = d.entries.iter().find(|e| e.name == "fut").unwrap();
    assert_eq!(fut.mtime, future);
    assert!(fut.newest_mtime <= fut.scanned_at);
    assert!(d.entry.newest_mtime <= d.entry.scanned_at);
    assert!(f.entry("").newest_mtime <= f.entry("").scanned_at);
}

#[test]
fn non_utf8_names_round_trip_through_path_raw() {
    let f = fx();
    let raw = f.root.join(std::ffi::OsStr::from_bytes(b"bad\xffname"));
    fs::create_dir(&raw).unwrap();
    fs::write(raw.join("inside"), vec![1u8; 8192]).unwrap();
    f.scan(false);

    let r = f.ls("");
    let e = &r.entries[0];
    let hex = e.path_raw.clone().expect("lossy path carries path_raw");
    assert_eq!(hex_decode(&hex).unwrap(), raw.as_os_str().as_bytes());
    let again = queries::ls(
        &f.snap(),
        &LsParams {
            path_raw: Some(hex),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(names(&again.entries), ["inside"]);

    let found = f.find(FindParams {
        name: Some("bad*".into()),
        ..Default::default()
    });
    assert_eq!(found.total.count, 1);
}

#[test]
fn unreadable_folder_marks_every_answer_incomplete() {
    if is_root() {
        return; // root reads everything
    }
    let f = fx();
    let now = now_secs();
    f.file("ok/a", 4096, now);
    f.file("locked/secret", 4096, now);
    let locked = f.p("locked");
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
    let rep = f.scan(false);
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(rep.denied, vec![f.s("locked")]);

    let r = f.ls("");
    assert!(r.header.incomplete);
    assert_eq!(r.header.unknown_subtrees, vec![f.s("locked")]);
    let l = r.entries.iter().find(|e| e.name == "locked").unwrap();
    assert!(l.denied);
    // D1 = B: unknown dates pass the untouched filter
    let t = f.top(TopParams {
        kind: Some("dir".into()),
        older_than: Some("30d".into()),
        ..Default::default()
    });
    assert!(t.entries.iter().any(|e| e.name == "locked"));
    assert!(t.header.incomplete);
}

#[test]
fn refresh_of_a_folder_now_a_file_points_at_the_parent() {
    let f = fx();
    let now = now_secs();
    f.file("tc/d/sub/a", 8192, now - DAY);
    f.scan(false);
    fs::remove_dir_all(f.p("tc/d")).unwrap();
    fs::write(f.p("tc/d"), vec![1u8; 4096]).unwrap();
    let e = refresh::run_refresh(RefreshRequest {
        path: f.p("tc/d").as_os_str().as_bytes().to_vec(),
        workers: 2,
        dir: f.dir.clone(),
        on_progress: None,
    })
    .unwrap_err();
    assert_eq!(e.code, ErrorCode::Usage, "{}", e.message);
    let parent = f.p("tc");
    assert_eq!(
        e.fix.as_deref(),
        Some(format!("sudo freespace refresh '{}'", parent.display()).as_str())
    );
    // the parent picks up the new kind
    f.refresh("tc");
    let d = f.entry("tc/d");
    assert_eq!(d.kind, Kind::File);
    assert_eq!(f.snap().meta.generation, 2);
}

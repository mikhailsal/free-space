//! The coarse backend end to end at the job level: real trees, real snapshot files, the v1
//! query layer on top. The folding threshold is a share of the filesystem, so the fixtures
//! size their files from it instead of assuming a disk.

use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use freespace_core::coarse::{self, Part, RefreshOutcome, ScanOutcome};
use freespace_core::job::{self, ScanRequest};
use freespace_core::platform;
use freespace_core::queries::{self, params::*, types::*};
use freespace_core::snapshot::reader::Snapshot;
use freespace_core::snapshot::{SnapshotDir, state};
use freespace_core::{ErrorCode, Result};

const KIB: u64 = 1024;
/// About the threshold the fixtures aim for; a small file stays far below it.
const TARGET: u64 = 192 * KIB;
/// A fixture on a filesystem whose threshold exceeds this would write absurd amounts.
const LARGEST_THRESHOLD: u64 = 64 * 1024 * KIB;

struct Fx {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    /// The coarse snapshot directory.
    dir: SnapshotDir,
    /// A full v1 snapshot of the same tree: the ground truth for totals.
    full: SnapshotDir,
    ppm: u32,
    threshold: u64,
}

/// `None` where the filesystem is too large for a small fixture to reach the threshold.
fn fx() -> Option<Fx> {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("r");
    fs::create_dir(&root).unwrap();
    let total = u128::from(platform::statvfs(&root).unwrap().total);
    let ppm = (u128::from(TARGET) * 1_000_000)
        .div_ceil(total)
        .clamp(1, 1_000_000) as u32;
    let threshold = (total * u128::from(ppm)).div_ceil(1_000_000).max(1) as u64;
    if threshold > LARGEST_THRESHOLD {
        eprintln!("skipped: the folding threshold of this filesystem is {threshold} bytes");
        return None;
    }
    let dir = SnapshotDir::new(tmp.path().join("coarse"));
    let full = SnapshotDir::new(tmp.path().join("full"));
    Some(Fx {
        _tmp: tmp,
        root,
        dir,
        full,
        ppm,
        threshold,
    })
}

/// Incompressible bytes: the same size on disk whatever the filesystem does.
fn noise(len: u64) -> Vec<u8> {
    let mut x = 0x9e37_79b9_7f4a_7c15u64;
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 24) as u8
        })
        .collect()
}

impl Fx {
    fn path(&self, rel: &str) -> PathBuf {
        if rel.is_empty() {
            self.root.clone()
        } else {
            self.root.join(rel)
        }
    }

    fn text(&self, rel: &str) -> String {
        self.path(rel).to_str().unwrap().to_owned()
    }

    fn file(&self, rel: &str, size: u64) {
        let path = self.path(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, noise(size)).unwrap();
    }

    /// A file well above the threshold.
    fn big(&self, rel: &str) {
        self.file(rel, 2 * self.threshold);
    }

    fn request(&self, roots: Vec<PathBuf>, ppm: u32, keep_previous: bool) -> coarse::ScanRequest {
        coarse::ScanRequest {
            roots,
            ppm,
            dir: self.dir.clone(),
            keep_previous,
            on_progress: None,
        }
    }

    fn scan(&self, keep_previous: bool) -> ScanOutcome {
        let request = self.request(vec![self.root.clone()], self.ppm, keep_previous);
        coarse::run_scan(request).unwrap()
    }

    fn refresh_path(&self, path: &Path) -> Result<RefreshOutcome> {
        coarse::run_refresh(coarse::RefreshRequest {
            path: path.as_os_str().as_bytes().to_vec(),
            dir: self.dir.clone(),
            on_progress: None,
        })
    }

    fn refresh(&self, rel: &str) -> Result<RefreshOutcome> {
        self.refresh_path(&self.path(rel))
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot::open_current(&self.dir).unwrap()
    }

    fn ls(&self, rel: &str) -> LsResponse {
        let params = LsParams {
            path: Some(self.text(rel)),
            limit: Some(1000),
            ..Default::default()
        };
        queries::ls(&self.snapshot(), &params).unwrap()
    }

    /// The root as a full v1 scan sees it right now.
    fn truth(&self) -> EntryOut {
        job::run_scan(ScanRequest {
            roots: vec![self.root.clone()],
            low_memory: false,
            one_file_system: true,
            workers: 2,
            dir: self.full.clone(),
            keep_previous: false,
            on_progress: None,
        })
        .unwrap();
        let snap = Snapshot::open_current(&self.full).unwrap();
        let params = LsParams {
            path: Some(self.text("")),
            ..Default::default()
        };
        queries::ls(&snap, &params).unwrap().entry
    }

    /// The coarse snapshot says what a full scan says about the root.
    fn agrees_with_truth(&self) {
        let (got, want) = (self.ls("").entry, self.truth());
        assert_eq!(got.allocated, want.allocated, "allocated");
        assert_eq!(got.apparent, want.apparent, "apparent");
        assert_eq!(got.items, want.items, "items");
        assert_eq!(got.errors, want.errors, "errors");
    }
}

fn names(ls: &LsResponse) -> Vec<&str> {
    ls.entries.iter().map(|e| e.name.as_str()).collect()
}

fn folded(ls: &LsResponse) -> Vec<&EntryOut> {
    ls.entries.iter().filter(|e| e.folded).collect()
}

/// A kept folder with a big file, a folder of small notes and a loose small file.
fn sample(f: &Fx) {
    f.big("big/huge.bin");
    for i in 0..20 {
        f.file(&format!("docs/note{i:02}.txt"), KIB);
    }
    f.file("loose.txt", KIB);
}

#[test]
fn a_scan_keeps_big_entries_folds_small_ones_and_keeps_totals_exact() {
    let Some(f) = fx() else { return };
    sample(&f);
    let out = f.scan(true);
    assert_eq!(out.report.generation, 1);
    assert_eq!(out.report.prev_generation, None);
    assert_eq!(out.folded_below, f.threshold);
    // the root, big, huge.bin and one bucket stand for 25 entries on disk
    assert_eq!(out.report.entries, 25);
    assert_eq!(out.rows, 4);
    f.agrees_with_truth();

    let ls = f.ls("");
    assert_eq!(ls.header.folded_below, Some(f.threshold));
    assert!(names(&ls).contains(&"big"), "{:?}", names(&ls));
    assert!(!names(&ls).contains(&"docs"), "{:?}", names(&ls));
    let buckets = folded(&ls);
    assert_eq!(buckets.len(), 1, "{:?}", names(&ls));
    assert!(buckets[0].name.starts_with("[small items"));
    // docs, its 20 notes and loose.txt
    assert_eq!(buckets[0].items, 22);
    assert!(buckets[0].allocated > 0 && buckets[0].allocated < f.threshold);
    // nothing else is listed, and the two rows hold all but the root's own block
    assert_eq!(ls.entries.len(), 2, "{:?}", names(&ls));
    let listed: u64 = ls.entries.iter().map(|e| e.allocated).sum();
    assert!(ls.entry.allocated >= listed);
    assert!(
        ls.entry.allocated - listed <= 64 * KIB,
        "{} of {}",
        listed,
        ls.entry.allocated
    );
}

#[test]
fn queries_list_what_is_kept_and_say_so_about_what_is_not() {
    let Some(f) = fx() else { return };
    sample(&f);
    f.scan(true);
    let snap = f.snapshot();
    let top = queries::top(
        &snap,
        &TopParams {
            path: Some(f.text("")),
            kind: Some("file".into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(top.entries[0].name, "huge.bin");
    assert!(top.entries.iter().all(|e| !e.folded), "{:?}", top.entries);
    assert_eq!(top.header.folded_below, Some(f.threshold));

    let found = |pattern: &str| {
        queries::find(
            &snap,
            &FindParams {
                path: Some(f.text("")),
                name: Some(pattern.into()),
                kind: Some("any".into()),
                ..Default::default()
            },
        )
        .unwrap()
    };
    assert_eq!(found("huge*").entries.len(), 1);
    assert!(
        found("note*").entries.is_empty(),
        "folded notes are not listed"
    );

    let missing = queries::ls(
        &snap,
        &LsParams {
            path: Some(f.text("docs/note03.txt")),
            ..Default::default()
        },
    )
    .unwrap_err();
    assert_eq!(missing.code, ErrorCode::PathNotFound);
    assert!(
        missing.message.contains("[small items]"),
        "{}",
        missing.message
    );

    let st = state::read(&f.dir).unwrap().unwrap();
    assert!(
        st.warnings.iter().any(|w| w.contains("coarse snapshot")),
        "{:?}",
        st.warnings
    );
}

#[test]
fn a_second_scan_keeps_the_first_as_prev_and_diff_sees_what_grew() {
    let Some(f) = fx() else { return };
    f.big("big/huge.bin");
    f.scan(true);
    f.big("more/huge2.bin");
    let second = f.scan(true);
    assert_eq!(second.report.generation, 2);
    assert_eq!(second.report.prev_generation, Some(1));
    assert!(f.dir.prev().exists());

    let diff = queries::diff(
        &f.dir,
        &DiffParams {
            path: Some(f.text("")),
            ..Default::default()
        },
    )
    .unwrap();
    let more = diff.entries.iter().find(|e| e.name == "more").unwrap();
    assert_eq!(more.status, DiffStatus::New);
    assert!(more.delta >= (2 * f.threshold) as i64, "{}", more.delta);
    assert_eq!(diff.header.folded_below, Some(f.threshold));

    let third = f.scan(false);
    assert_eq!(third.report.generation, 3);
    assert_eq!(third.report.prev_generation, None);
    assert!(!f.dir.prev().exists());
}

#[test]
fn refreshing_a_kept_folder_matches_a_fresh_scan() {
    let Some(f) = fx() else { return };
    f.big("big/huge.bin");
    f.file("big/inner/small.txt", KIB);
    f.file("docs/note.txt", KIB);
    f.scan(true);
    f.big("big/second.bin");
    fs::remove_file(f.path("big/inner/small.txt")).unwrap();
    f.file("docs/changed-but-not-refreshed.txt", KIB);

    let out = f.refresh("big").unwrap();
    assert_eq!(out.rescanned.part, Part::Folder);
    assert_eq!(out.rescanned.path, f.path("big"));
    assert!(!out.report.removed);
    assert_eq!(out.report.generation, 2);
    assert!(f.ls("big").entries.iter().any(|e| e.name == "second.bin"));

    // the refresh did not look at docs: the root differs from the disk by exactly that file
    let (got, want) = (f.ls("").entry, f.truth());
    assert_eq!(want.items, got.items + 1);
    // refreshing the root rescans all of it and brings everything up to date
    let all = f.refresh("").unwrap();
    assert_eq!(all.rescanned.part, Part::Root);
    f.agrees_with_truth();
}

#[test]
fn refreshing_a_folded_folder_rescans_the_bucket_that_holds_it() {
    let Some(f) = fx() else { return };
    f.big("big/huge.bin");
    f.file("docs/note.txt", KIB);
    f.scan(true);
    f.file("docs/another.txt", KIB);
    f.file("loose.txt", KIB);

    let out = f.refresh("docs").unwrap();
    assert_eq!(out.rescanned.part, Part::Bucket);
    assert_eq!(out.rescanned.path, f.path(""));
    assert!(out.report.generation == 2 && !out.report.removed);
    f.agrees_with_truth();
}

#[test]
fn refreshing_a_vanished_folder_drops_it() {
    let Some(f) = fx() else { return };
    f.big("big/huge.bin");
    f.big("keep/huge.bin");
    f.file("docs/note.txt", KIB);
    f.scan(true);
    fs::remove_dir_all(f.path("big")).unwrap();

    let out = f.refresh("big").unwrap();
    assert!(out.report.removed);
    assert!(!names(&f.ls("")).contains(&"big"));
    f.agrees_with_truth();
    // a folded folder that vanished is dropped from its bucket
    fs::remove_dir_all(f.path("docs")).unwrap();
    let out = f.refresh("docs").unwrap();
    assert!(out.report.removed);
    assert_eq!(out.rescanned.part, Part::Bucket);
    f.agrees_with_truth();
}

#[test]
fn a_refresh_of_a_region_with_hardlinks_rescans_the_whole_root() {
    let Some(f) = fx() else { return };
    f.big("big/huge.bin");
    f.file("docs/note.txt", KIB);
    fs::hard_link(f.path("big/huge.bin"), f.path("docs/alias.bin")).unwrap();
    f.scan(true);
    f.agrees_with_truth();
    f.file("docs/another.txt", KIB);

    let out = f.refresh("docs").unwrap();
    assert_eq!(out.rescanned.part, Part::Root);
    assert_eq!(out.rescanned.path, f.path(""));
    f.agrees_with_truth();
}

#[test]
fn refresh_refuses_what_it_cannot_do_and_says_why() {
    let Some(f) = fx() else { return };
    let none = f.refresh("").map(|_| ()).unwrap_err();
    assert_eq!(none.code, ErrorCode::NoSnapshot, "{}", none.message);

    f.big("big/huge.bin");
    f.scan(true);
    let outside = f
        .refresh_path(f.root.parent().unwrap())
        .map(|_| ())
        .unwrap_err();
    assert_eq!(outside.code, ErrorCode::PathNotFound, "{}", outside.message);
    let file = f.refresh("big/huge.bin").map(|_| ()).unwrap_err();
    assert_eq!(file.code, ErrorCode::Usage, "{}", file.message);
    assert!(file.message.contains("not a folder"), "{}", file.message);
    // refusals leave the snapshot as it was
    assert_eq!(f.snapshot().meta.generation, 1);
}

#[test]
fn a_scan_refuses_bad_arguments_before_touching_anything() {
    let Some(f) = fx() else { return };
    for ppm in [0, 1_000_001] {
        let request = f.request(vec![f.root.clone()], ppm, true);
        let e = coarse::run_scan(request).map(|_| ()).unwrap_err();
        assert_eq!(e.code, ErrorCode::Usage, "{ppm}: {}", e.message);
    }
    let request = f.request(vec![f.path("does-not-exist")], f.ppm, true);
    let e = coarse::run_scan(request).map(|_| ()).unwrap_err();
    assert_eq!(e.code, ErrorCode::Usage, "{}", e.message);
    let request = f.request(Vec::new(), f.ppm, true);
    let e = coarse::run_scan(request).map(|_| ()).unwrap_err();
    assert_eq!(e.code, ErrorCode::Config, "{}", e.message);
    assert!(Snapshot::open_current(&f.dir).is_err());
}

#[test]
fn an_unreadable_folder_makes_the_snapshot_incomplete() {
    use std::os::unix::fs::PermissionsExt;
    if unsafe { libc::geteuid() } == 0 {
        return;
    }
    let Some(f) = fx() else { return };
    f.big("big/huge.bin");
    f.file("locked/secret.txt", KIB);
    let locked = f.path("locked");
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
    let out = f.scan(true);
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();

    assert!(out.report.errors >= 1);
    let ls = f.ls("");
    assert!(ls.header.incomplete);
    assert_eq!(ls.header.unknown_subtrees, vec![f.text("locked")]);
    let st = state::read(&f.dir).unwrap().unwrap();
    assert!(
        st.warnings.iter().any(|w| w.contains("locked")),
        "{:?}",
        st.warnings
    );
    // an unreadable folder is kept as it is, however small
    let entry = ls.entries.iter().find(|e| e.name == "locked").unwrap();
    assert!(entry.denied, "{entry:?}");
}

#[test]
fn a_scan_leaves_no_temporary_files_and_a_v1_directory_alone() {
    let Some(f) = fx() else { return };
    sample(&f);
    f.scan(true);
    f.scan(true);
    let mut left: Vec<String> = fs::read_dir(&f.dir.path)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    left.sort();
    assert!(left.iter().all(|n| !n.contains("tmp")), "{left:?}");
    assert!(left.iter().any(|n| n == "snapshot.db"), "{left:?}");
    assert!(
        !f.full.path.exists(),
        "the coarse scan must not create a v1 directory"
    );
}

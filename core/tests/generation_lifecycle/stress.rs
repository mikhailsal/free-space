use super::support::Fixture;
use std::{fs, os::unix::ffi::OsStrExt};

#[test]
fn repeated_refresh_rotation_reader_and_bounded_maintenance_cycles() {
    let mut f = Fixture::new(64);
    let mut high_water = Vec::new();
    let mut work = Vec::new();
    for epoch in 0..8 {
        let pinned = f.reader();
        let tree = pinned
            .subtree(f.root.as_os_str().as_bytes(), false)
            .unwrap();
        let pair = pinned.generations;
        let previous = pinned.subtree(f.root.as_os_str().as_bytes(), true).unwrap();
        refresh_epoch(&mut f, epoch, &mut work);
        // Two rotations release refresh baselines only after the reader closes.
        for disk in [false, true] {
            f.store
                .scan_roots_with_options(&[f.root.clone()], true, 1, disk, true)
                .unwrap();
        }
        assert_eq!(pinned.generations, pair);
        assert_eq!(
            pinned
                .subtree(f.root.as_os_str().as_bytes(), false)
                .unwrap(),
            tree
        );
        assert_eq!(
            pinned.subtree(f.root.as_os_str().as_bytes(), true).unwrap(),
            previous
        );
        drop(pinned);
        let before = f.integrity_and_accounting();
        let current = f.reader();
        let expected = current
            .subtree(f.root.as_os_str().as_bytes(), false)
            .unwrap();
        let previous = current
            .subtree(f.root.as_os_str().as_bytes(), true)
            .unwrap();
        let pair = current.generations;
        let history = current.refresh_history().unwrap();
        drop(current);
        let deleted: usize = (0..60).map(|_| f.upkeep(23, false).deleted).sum();
        let after = f.integrity_and_accounting();
        assert!(
            deleted > 0 && after < before,
            "epoch {epoch}: {before} -> {after}"
        );
        assert!(
            after <= 4 * expected.len() as u64 + 100,
            "retention failed to plateau: {after}"
        );
        let current = f.reader();
        assert_eq!(current.generations, pair);
        assert_eq!(
            current
                .subtree(f.root.as_os_str().as_bytes(), false)
                .unwrap(),
            expected
        );
        assert_eq!(
            current
                .subtree(f.root.as_os_str().as_bytes(), true)
                .unwrap(),
            previous
        );
        assert_eq!(current.refresh_history().unwrap(), history);
        drop(current);
        let checkpoint = f.upkeep(23, true);
        assert!(!checkpoint.checkpoint_busy);
        assert_eq!(checkpoint.wal_bytes, 0);
        high_water.push(after);
    }
    let min = work.iter().map(|(_, steps)| *steps).min().unwrap();
    let max = work.iter().map(|(_, steps)| *steps).max().unwrap();
    assert!(
        max < min * 3,
        "refresh SQL grew with retained history: {min}..{max}"
    );
    eprintln!(
        "160 refreshes, 16 rotations, bounded GC: retained={high_water:?}; refresh VM steps={min}..{max}"
    );
}

fn refresh_epoch(f: &mut Fixture, epoch: usize, work: &mut Vec<(u64, u64)>) {
    for cycle in 0..20 {
        fs::write(f.root.join("local/a"), vec![1; 1 + cycle % 7]).unwrap();
        let index = epoch * 20 + cycle;
        let old = f.root.join(format!("local/transient-{}", (index + 2) % 3));
        if old.exists() {
            fs::remove_file(old).unwrap();
        }
        fs::write(f.root.join(format!("local/transient-{}", index % 3)), [1]).unwrap();
        work.push(f.measured_refresh());
        if cycle % 5 == 0 {
            let deferred = f.store.maintain(23, true).unwrap();
            assert!(deferred.readers_blocking);
            assert_eq!((deferred.examined, deferred.deleted), (0, 0));
        }
    }
}

#[test]
fn repeated_local_refresh_sql_work_stays_local_as_unrelated_width_grows() {
    let mut measurements = Vec::new();
    for width in [8, 1024] {
        let mut f = Fixture::new(width);
        let mut rounds = Vec::new();
        for n in 0..12 {
            fs::write(f.root.join("local/a"), vec![1; n + 2]).unwrap();
            rounds.push(f.measured_refresh());
        }
        f.integrity_and_accounting();
        measurements.push(rounds);
    }
    for (small, large) in measurements[0].iter().zip(&measurements[1]) {
        assert_eq!(
            small.0, large.0,
            "SQL writes expanded outside local subtree"
        );
        assert!(
            large.1 < small.1 * 2,
            "unrelated width caused global SQL work: {small:?} -> {large:?}"
        );
    }
    eprintln!("12 local refreshes, unrelated width 8 vs 1024: {measurements:?}");
}

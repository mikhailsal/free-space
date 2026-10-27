use super::*;
use clap::Parser;

fn available(bytes: u64) -> MemoryAvailability {
    MemoryAvailability {
        available_bytes: Some(bytes),
        limited: false,
    }
}

#[test]
fn explicit_and_saved_choices_skip_auto_probes() {
    for value in [false, true] {
        let choice = if value { Choice::True } else { Choice::False };
        let explicit = choose(Some(choice), Some(!value), || {
            panic!("explicit choice probed host")
        });
        assert_eq!(explicit.low_memory, value);
        assert!(explicit.reason.starts_with("explicit:"));
        let saved = choose(None, Some(value), || panic!("saved choice probed host"));
        assert_eq!(saved.low_memory, value);
        assert!(saved.reason.starts_with("preserved:"));
    }
}

#[test]
fn missing_choice_and_requested_auto_recalculate() {
    for (choice, configured) in [
        (None, None),
        (Some(Choice::Auto), Some(false)),
        (Some(Choice::Auto), Some(true)),
    ] {
        let decision = choose(choice, configured, || selected(true, "auto: test probe"));
        assert!(decision.low_memory);
        assert_eq!(decision.reason, "auto: test probe");
    }
}

#[test]
fn memory_estimate_has_exact_quarter_budget_boundaries() {
    for (backend, per_entry) in [(Backend::V1, 384), (Backend::V2, 1024)] {
        let estimate = 64 * 1024 * 1024 + 100 * per_entry;
        assert!(!automatic(backend, available(estimate * 4), Some(100)).low_memory);
        assert!(automatic(backend, available(estimate * 4 - 1), Some(100)).low_memory);
    }
    assert!(automatic(Backend::V2, available(128 * 1024 * 1024), Some(4096)).low_memory);
    assert!(!automatic(Backend::V2, available(4 * 1024 * 1024 * 1024), Some(4096)).low_memory);
}

#[test]
fn uncertainty_and_arithmetic_overflow_select_disk_staging() {
    let unknown = MemoryAvailability {
        available_bytes: None,
        limited: true,
    };
    assert!(automatic(Backend::V1, unknown, Some(1)).low_memory);
    assert!(automatic(Backend::V1, available(u64::MAX), None).low_memory);
    assert!(automatic(Backend::V1, available(u64::MAX), Some(u64::MAX)).low_memory);
    let addition_overflow = (u64::MAX - 64 * 1024 * 1024) / 1024 + 1;
    assert!(automatic(Backend::V2, available(u64::MAX), Some(addition_overflow)).low_memory);
    assert!(automatic(Backend::V1, available(0), Some(0)).low_memory);
    let between_backends = available(512 * 1024 * 1024);
    assert!(!automatic(Backend::V1, between_backends, Some(100_000)).low_memory);
    assert!(automatic(Backend::V2, between_backends, Some(100_000)).low_memory);
}

#[test]
fn used_inodes_require_every_filesystem_and_count_devices_once() {
    let first = Inodes {
        dev: 1,
        total: 100,
        free: 40,
    };
    let second = Inodes {
        dev: 2,
        total: 80,
        free: 10,
    };
    assert_eq!(sum_entries(&[first, first, second]), Some(130));
    assert_eq!(
        sum_entries(&[
            first,
            Inodes {
                dev: 2,
                total: 0,
                free: 0
            }
        ]),
        None
    );
    assert_eq!(
        sum_entries(&[Inodes {
            dev: 1,
            total: 10,
            free: 11
        }]),
        None
    );
    assert_eq!(
        sum_entries(&[
            Inodes {
                dev: 1,
                total: u64::MAX,
                free: 0
            },
            Inodes {
                dev: 2,
                total: 1,
                free: 0
            }
        ]),
        None
    );
    assert_eq!(estimate_entries(&[]), None);
    let directory = tempfile::tempdir().unwrap();
    assert_eq!(estimate_entries(&[directory.path().join("missing")]), None);
    let unsafe_mount = MountInfo {
        mount_point: "/".into(),
        fs_type: "nfs".into(),
        source: "server:/".into(),
        dev: "0:1".into(),
        root: "/".into(),
    };
    assert!(root_inodes(Path::new("/"), &[unsafe_mount]).is_none());
}

#[test]
fn choice_parser_accepts_bare_true_values_and_rejects_other_values() {
    for (arguments, expected) in [
        (vec!["freespace", "install-timer"], None),
        (
            vec!["freespace", "install-timer", "--low-memory"],
            Some(Choice::True),
        ),
        (
            vec!["freespace", "install-timer", "--low-memory", "--no-start"],
            Some(Choice::True),
        ),
        (
            vec!["freespace", "install-timer", "--low-memory", "true"],
            Some(Choice::True),
        ),
        (
            vec!["freespace", "install-timer", "--low-memory", "false"],
            Some(Choice::False),
        ),
        (
            vec!["freespace", "install-timer", "--low-memory", "auto"],
            Some(Choice::Auto),
        ),
    ] {
        let cli = crate::Cli::try_parse_from(arguments).unwrap();
        let Some(crate::Command::InstallTimer(install)) = cli.command else {
            panic!("not install-timer")
        };
        assert_eq!(install.low_memory, expected);
    }
    assert!(
        crate::Cli::try_parse_from(["freespace", "install-timer", "--low-memory", "sometimes"])
            .is_err()
    );
}

use super::*;
use std::collections::HashMap;

const ROOT: &str = "1 0 8:1 / / rw - ext4 /dev/root rw\n";
const V2: &str = "2 1 0:26 / /sys/fs/cgroup rw - cgroup2 cgroup rw\n";
const V1: &str = "2 1 0:26 / /sys/fs/cgroup/memory rw - cgroup memory rw,memory\n";

struct Fixture {
    files: HashMap<PathBuf, Result<String, io::ErrorKind>>,
}

impl Fixture {
    fn new(cgroup: &str, mounts: &str) -> Self {
        let mut fixture = Self {
            files: HashMap::new(),
        };
        fixture.set(
            "/proc/meminfo",
            "MemTotal: 8388608 kB\nMemAvailable: 6291456 kB\n",
        );
        fixture.set("/proc/self/cgroup", cgroup);
        fixture.set("/proc/self/mountinfo", &format!("{ROOT}{mounts}"));
        fixture
    }

    fn v2(group: &str) -> Self {
        let mut fixture = Self::new(&format!("0::{group}\n"), V2);
        fixture.set("/sys/fs/cgroup/memory.max", "max");
        fixture
    }

    fn v1(group: &str) -> Self {
        let mut fixture = Self::new(&format!("5:cpu,memory:{group}\n"), V1);
        fixture.set(
            "/sys/fs/cgroup/memory/memory.limit_in_bytes",
            "9223372036854771712",
        );
        fixture
    }

    fn set(&mut self, path: impl AsRef<Path>, text: &str) {
        self.files
            .insert(path.as_ref().to_path_buf(), Ok(text.into()));
    }

    fn deny(&mut self, path: &str) {
        self.files
            .insert(path.into(), Err(io::ErrorKind::PermissionDenied));
    }

    fn probe(&self) -> MemoryAvailability {
        probe(|path| match self.files.get(path) {
            Some(Ok(text)) => Ok(text.clone()),
            Some(Err(kind)) => Err(io::Error::from(*kind)),
            None => Err(io::Error::from(io::ErrorKind::NotFound)),
        })
    }
}

fn available(bytes: u64, limited: bool) -> MemoryAvailability {
    MemoryAvailability {
        available_bytes: Some(bytes),
        limited,
    }
}

fn unknown() -> MemoryAvailability {
    MemoryAvailability {
        available_bytes: None,
        limited: true,
    }
}

#[test]
fn root_and_unlimited_hierarchy_use_host_available() {
    assert_eq!(Fixture::v2("/").probe(), available(6 << 30, false));
    assert_eq!(Fixture::v1("/").probe(), available(6 << 30, false));
    let mut root_without_controller_files = Fixture::v2("/");
    root_without_controller_files
        .files
        .remove(Path::new("/sys/fs/cgroup/memory.max"));
    assert_eq!(
        root_without_controller_files.probe(),
        available(6 << 30, false)
    );
    assert_eq!(Fixture::new("", "").probe(), available(6 << 30, false));
}

#[test]
fn v2_uses_remaining_limit_instead_of_host_free_memory() {
    let mut fixture = Fixture::v2("/job");
    fixture.set("/sys/fs/cgroup/job/memory.max", "4294967296\n");
    fixture.set("/sys/fs/cgroup/job/memory.current", "1073741824\n");
    assert_eq!(fixture.probe(), available(3 << 30, true));
    fixture.set("/proc/meminfo", "MemTotal: 100 kB\nMemAvailable: 25 kB\n");
    assert_eq!(fixture.probe(), available(25 * 1024, true));
}

#[test]
fn ancestor_remaining_usage_includes_siblings() {
    let mut fixture = Fixture::v2("/parent/job");
    fixture.set("/sys/fs/cgroup/parent/job/memory.max", "10000");
    fixture.set("/sys/fs/cgroup/parent/job/memory.current", "1000");
    fixture.set("/sys/fs/cgroup/parent/memory.max", "5000");
    fixture.set("/sys/fs/cgroup/parent/memory.current", "4000");
    assert_eq!(fixture.probe(), available(1000, true));
    fixture.set("/sys/fs/cgroup/parent/job/memory.max", "max");
    assert_eq!(fixture.probe(), available(1000, true));
}

#[test]
fn v1_memory_controller_is_selected_in_hybrid_hierarchy() {
    let mut fixture = Fixture::v1("/job");
    fixture.set("/proc/self/cgroup", "0::/unified\n5:cpu,memory:/job\n");
    fixture.set("/sys/fs/cgroup/memory/job/memory.limit_in_bytes", "1000");
    fixture.set("/sys/fs/cgroup/memory/job/memory.usage_in_bytes", "200");
    assert_eq!(fixture.probe(), available(800, true));
}

#[test]
fn v1_ancestor_cap_is_not_hidden_by_unlimited_leaf() {
    let mut fixture = Fixture::v1("/parent/job");
    fixture.set(
        "/sys/fs/cgroup/memory/parent/job/memory.limit_in_bytes",
        "9223372036854771712",
    );
    fixture.set("/sys/fs/cgroup/memory/parent/memory.limit_in_bytes", "1000");
    fixture.set("/sys/fs/cgroup/memory/parent/memory.usage_in_bytes", "500");
    assert_eq!(fixture.probe(), available(500, true));
}

#[test]
fn usage_over_limit_and_zero_limit_saturate_at_zero() {
    for limit in ["0", "1000"] {
        let mut fixture = Fixture::v2("/job");
        fixture.set("/sys/fs/cgroup/job/memory.max", limit);
        fixture.set("/sys/fs/cgroup/job/memory.current", "2000");
        assert_eq!(fixture.probe(), available(0, true));
    }
}

#[test]
fn finite_unreadable_or_invalid_limit_and_usage_are_unknown() {
    for value in ["invalid", "-1", "18446744073709551616"] {
        let mut fixture = Fixture::v2("/job");
        fixture.set("/sys/fs/cgroup/job/memory.max", value);
        fixture.set("/sys/fs/cgroup/job/memory.current", "0");
        assert_eq!(fixture.probe(), unknown());
    }
    let mut fixture = Fixture::v2("/job");
    fixture.set("/sys/fs/cgroup/job/memory.max", "1024");
    assert_eq!(fixture.probe(), unknown());
    fixture.deny("/sys/fs/cgroup/job/memory.current");
    assert_eq!(fixture.probe(), unknown());
    fixture.set("/sys/fs/cgroup/job/memory.current", "0");
    fixture.deny("/sys/fs/cgroup/job/memory.max");
    assert_eq!(fixture.probe(), unknown());
}

#[test]
fn unknown_ancestor_cannot_be_ignored_after_readable_leaf() {
    let mut fixture = Fixture::v2("/parent/job");
    fixture.set("/sys/fs/cgroup/parent/job/memory.max", "1024");
    fixture.set("/sys/fs/cgroup/parent/job/memory.current", "0");
    fixture.deny("/sys/fs/cgroup/parent/memory.max");
    assert_eq!(fixture.probe(), unknown());
}

#[test]
fn unavailable_host_estimate_does_not_become_entire_cap() {
    let mut fixture = Fixture::v2("/job");
    fixture.set("/sys/fs/cgroup/job/memory.max", "1024");
    fixture.set("/sys/fs/cgroup/job/memory.current", "0");
    fixture.deny("/proc/meminfo");
    assert_eq!(fixture.probe(), unknown());
}

#[test]
fn meminfo_units_and_multiplication_are_checked() {
    for text in [
        "MemTotal: 100 kB\n",
        "MemTotal: 100 kB\nMemAvailable: 10 MB\n",
        "MemTotal: 18446744073709551615 kB\nMemAvailable: 10 kB\n",
        "MemTotal: 100 kB\nMemAvailable: 18446744073709551615 kB\n",
        "MemTotal: 100 kB\nMemAvailable: 10 kB extra\n",
    ] {
        assert_eq!(host_available(text), None);
    }
    assert_eq!(
        host_available("MemTotal: 100 kB\nMemAvailable: 200 kB\n"),
        Some(100 * 1024)
    );
}

#[test]
fn bind_mount_root_is_removed_before_mapping_group() {
    let mut fixture = Fixture::new(
        "0::/tenant/job\n",
        "2 1 0:26 /tenant /sys/cgroup rw - cgroup2 cgroup rw\n",
    );
    fixture.set("/sys/cgroup/job/memory.max", "1000");
    fixture.set("/sys/cgroup/job/memory.current", "100");
    fixture.set("/sys/cgroup/memory.max", "max");
    assert_eq!(fixture.probe(), available(900, true));
}

#[test]
fn private_namespace_root_maps_to_visible_subtree_mount() {
    let mut fixture = Fixture::new(
        "0::/\n",
        "2 1 0:26 /docker/container /sys/cgroup rw - cgroup2 cgroup rw\n",
    );
    fixture.set("/sys/cgroup/memory.max", "1000");
    fixture.set("/sys/cgroup/memory.current", "200");
    assert_eq!(fixture.probe(), available(800, true));
}

#[test]
fn escaped_mount_point_and_nonroot_mount_ancestor_are_supported() {
    let mut fixture = Fixture::new(
        "0::/tenant/job\n",
        "2 1 0:26 /tenant /sys/cgroup\\040memory rw - cgroup2 cgroup rw\n",
    );
    fixture.set("/sys/cgroup memory/job/memory.max", "max");
    fixture.set("/sys/cgroup memory/memory.max", "1000");
    fixture.set("/sys/cgroup memory/memory.current", "750");
    assert_eq!(fixture.probe(), available(250, true));
}

#[test]
fn broader_mount_preserves_visible_parent_limit() {
    let mut fixture = Fixture::v2("/job");
    fixture.set(
        "/proc/self/mountinfo",
        &format!("{ROOT}{V2}3 1 0:26 /job /bound rw - cgroup2 cgroup rw\n"),
    );
    fixture.set("/sys/fs/cgroup/job/memory.max", "10000");
    fixture.set("/sys/fs/cgroup/job/memory.current", "0");
    fixture.set("/sys/fs/cgroup/memory.max", "1000");
    fixture.set("/sys/fs/cgroup/memory.current", "900");
    fixture.set("/bound/memory.max", "max");
    assert_eq!(fixture.probe(), available(100, true));
}

#[test]
fn hidden_mount_is_not_probed_through_its_overmount() {
    let mut fixture = Fixture::v2("/");
    fixture.set(
        "/proc/self/mountinfo",
        &format!("{ROOT}{V2}3 2 0:55 / /sys/fs/cgroup rw - tmpfs tmpfs rw\n"),
    );
    assert_eq!(fixture.probe(), unknown());
}

#[test]
fn missing_disabled_controller_still_checks_parent() {
    let mut fixture = Fixture::v2("/parent/job");
    fixture.set("/sys/fs/cgroup/parent/job/cgroup.controllers", "cpu io\n");
    fixture.set("/sys/fs/cgroup/parent/memory.max", "1000");
    fixture.set("/sys/fs/cgroup/parent/memory.current", "900");
    assert_eq!(fixture.probe(), available(100, true));
    fixture.set("/sys/fs/cgroup/parent/job/cgroup.controllers", "memory cpu");
    assert_eq!(fixture.probe(), unknown());
}

#[test]
fn malformed_mapping_and_proc_errors_fail_conservatively() {
    for membership in ["0::relative\n", "0::/../outside\n", "0::/\n0::/job\n"] {
        let fixture = Fixture::new(membership, V2);
        assert_eq!(fixture.probe(), unknown());
    }
    let mut fixture = Fixture::v2("/job");
    fixture.set(
        "/proc/self/mountinfo",
        &format!("{ROOT}2 1 0:26 /jo /sys/cgroup rw - cgroup2 cgroup rw\n"),
    );
    assert_eq!(fixture.probe(), unknown());
    fixture.deny("/proc/self/mountinfo");
    assert_eq!(fixture.probe(), unknown());
    fixture.deny("/proc/self/cgroup");
    assert_eq!(fixture.probe(), unknown());
}

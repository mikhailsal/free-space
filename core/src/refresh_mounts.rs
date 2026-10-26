//! Refresh must retain the enclosing scan root's live mount boundary.
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use crate::error::{Error, Result};
use crate::platform::{self, MountInfo};
use crate::scan::RootInfo;

pub(super) fn check(
    target: &Path,
    target_dev: Option<u64>,
    roots: &[RootInfo],
    one_file_system: bool,
    label: &str,
) -> Result<()> {
    let root = enclosing_root(target, roots.iter().map(|r| r.path.as_path()))
        .ok_or_else(|| Error::generic("refresh target has no enclosing snapshot root"))?;
    // An explicitly configured root establishes its own filesystem boundary.
    if target == root {
        return Ok(());
    }
    let root_dev = root
        .metadata()
        .map_err(|e| Error::io(format!("stat scan root {}", root.display()), e))?
        .dev();
    if outside_boundary(
        target,
        target_dev,
        root,
        root_dev,
        one_file_system,
        &platform::visible_mountinfo_entries(),
    ) {
        return Err(Error::usage(format!(
            "{label} is a mount point outside this root's filesystem; refresh is per filesystem"
        )));
    }
    Ok(())
}

fn enclosing_root<'a>(target: &Path, roots: impl Iterator<Item = &'a Path>) -> Option<&'a Path> {
    roots
        .filter(|root| target.starts_with(root))
        .max_by_key(|root| root.as_os_str().len())
}

fn outside_boundary(
    target: &Path,
    target_dev: Option<u64>,
    root: &Path,
    root_dev: u64,
    one_file_system: bool,
    mounts: &[platform::MountInfoEntry],
) -> bool {
    if target == root {
        return false;
    }
    let Some(root_mount) = mounts
        .iter()
        .filter(|entry| {
            target.starts_with(&entry.mount.mount_point)
                && root.starts_with(&entry.mount.mount_point)
        })
        .max_by_key(|entry| entry.mount.mount_point.as_os_str().len())
    else {
        return target_dev.is_some_and(|dev| one_file_system && dev != root_dev);
    };
    let root_mount = root_mount.clone();
    let mut root_and_visible_descendants = vec![root_mount.clone()];
    root_and_visible_descendants.extend(
        mounts
            .iter()
            .filter(|entry| {
                entry.mount.mount_point != root_mount.mount.mount_point
                    && entry.mount.mount_point.starts_with(root)
                    && target.starts_with(&entry.mount.mount_point)
            })
            .cloned(),
    );
    let root_dev_string = crate::scan::walk::dev_string(root_dev);
    let foreign_mount_ancestor = one_file_system
        && root_and_visible_descendants
            .iter()
            .any(|entry| entry.mount.mount_point != root && entry.mount.dev != root_dev_string);
    let visible_mounts: Vec<MountInfo> = root_and_visible_descendants
        .iter()
        .map(|entry| entry.mount.clone())
        .collect();
    let subtree_bind_ancestor =
        platform::bind_mounts_below(&visible_mounts, root, &root_dev_string)
            .iter()
            .any(|mount| target.starts_with(mount));
    foreign_mount_ancestor
        || target_dev.is_some_and(|dev| one_file_system && dev != root_dev)
        || subtree_bind_ancestor
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    fn mounts() -> Vec<platform::MountInfoEntry> {
        platform::parse_mountinfo_entries(
            b"1 0 8:3 / / rw - ext4 /dev/sda rw\n\
             2 1 0:22 / /data/late rw - tmpfs tmpfs rw\n\
             3 1 8:3 /elsewhere /data/bind rw - ext4 /dev/sda rw\n\
             4 1 8:3 /data/self /data/self rw - ext4 /dev/sda rw\n",
        )
    }

    #[test]
    fn live_root_stat_errors_are_not_a_deletion_or_device_zero() {
        let dir = tempfile::tempdir().unwrap();
        for path in [
            dir.path().join("missing"),
            Path::new(OsStr::from_bytes(b"/invalid\0root")).to_path_buf(),
        ] {
            let root = RootInfo {
                path,
                dev: 0,
                fs_type: String::new(),
                stats: Default::default(),
            };
            let target = root.path.join("child");
            let error = check(&target, None, &[root], true, "child").unwrap_err();
            assert_eq!(error.code, crate::ErrorCode::Generic);
            assert!(error.message.contains("stat scan root"));
        }
    }

    #[test]
    fn late_mounts_keep_the_original_scan_boundary() {
        let mounts = mounts();
        for target in ["/data/late", "/data/late/child"] {
            assert!(outside_boundary(
                Path::new(target),
                Some(22),
                Path::new("/data"),
                0x803,
                true,
                &mounts
            ));
            assert!(!outside_boundary(
                Path::new(target),
                Some(22),
                Path::new("/data"),
                0x803,
                false,
                &mounts
            ));
        }
        for cross in [false, true] {
            for target in ["/data/bind", "/data/bind/child"] {
                assert!(outside_boundary(
                    Path::new(target),
                    Some(0x803),
                    Path::new("/data"),
                    0x803,
                    cross,
                    &mounts
                ));
            }
            for target in ["/data/self", "/data/self/child", "/data/binder"] {
                assert!(!outside_boundary(
                    Path::new(target),
                    Some(0x803),
                    Path::new("/data"),
                    0x803,
                    cross,
                    &mounts
                ));
            }
        }
    }

    #[test]
    fn configured_roots_and_raw_component_boundaries() {
        let roots = [Path::new("/"), Path::new("/data"), Path::new("/data/late")];
        assert_eq!(
            enclosing_root(Path::new("/data/late/child"), roots.into_iter()),
            Some(Path::new("/data/late"))
        );
        assert_eq!(
            enclosing_root(Path::new("/database"), roots.into_iter()),
            Some(Path::new("/"))
        );
        assert!(!outside_boundary(
            Path::new("/data/late"),
            Some(22),
            Path::new("/data/late"),
            22,
            true,
            &mounts()
        ));
        let raw = Path::new(OsStr::from_bytes(b"/data/\xff"));
        let mut mounts = mounts();
        mounts.push(platform::MountInfoEntry {
            mount_id: 5,
            parent_id: 1,
            mount: MountInfo {
                mount_point: raw.into(),
                root: "/other".into(),
                dev: "8:3".into(),
                fs_type: "ext4".into(),
                source: "disk".into(),
            },
        });
        assert!(outside_boundary(
            &raw.join("child"),
            Some(0x803),
            Path::new("/data"),
            0x803,
            true,
            &mounts
        ));
        assert!(!outside_boundary(
            Path::new(OsStr::from_bytes(b"/data/\xffsibling")),
            Some(0x803),
            Path::new("/data"),
            0x803,
            true,
            &mounts
        ));
    }

    #[test]
    fn missing_targets_check_known_mount_ancestors_before_deletion() {
        let mounts = mounts();
        for one_file_system in [false, true] {
            assert!(outside_boundary(
                Path::new("/data/bind/missing"),
                None,
                Path::new("/data"),
                0x803,
                one_file_system,
                &mounts,
            ));
        }
        assert!(outside_boundary(
            Path::new("/data/late/missing"),
            None,
            Path::new("/data"),
            0x803,
            true,
            &mounts,
        ));
        assert!(!outside_boundary(
            Path::new("/data/ordinary/missing"),
            None,
            Path::new("/data"),
            0x803,
            true,
            &mounts,
        ));
        assert!(outside_boundary(
            Path::new("/data/unlisted-mount"),
            Some(22),
            Path::new("/data"),
            0x803,
            true,
            &[],
        ));
    }

    #[test]
    fn reentered_root_device_does_not_hide_a_foreign_mount_ancestor() {
        let mounts = platform::parse_mountinfo_entries(
            b"1 0 8:3 / / rw - ext4 /dev/sda rw\n\
             2 1 0:22 / /data/late rw - tmpfs tmpfs rw\n\
             3 2 8:3 /data/late/child /data/late/child rw - ext4 /dev/sda rw\n",
        );
        assert!(outside_boundary(
            Path::new("/data/late/child"),
            Some(0x803),
            Path::new("/data"),
            0x803,
            true,
            &mounts,
        ));
        assert!(!outside_boundary(
            Path::new("/data/late/child"),
            Some(0x803),
            Path::new("/data"),
            0x803,
            false,
            &mounts,
        ));
    }

    #[test]
    fn hidden_mount_layers_and_their_descendants_are_not_visible_boundaries() {
        // Deliberately shuffled: visibility follows mount IDs and parent IDs,
        // not the order of rows in mountinfo.
        let entries = platform::parse_mountinfo_entries(
            b"4 2 8:3 /data/late/child /data/late/child rw - ext4 /dev/sda rw\n\
              3 2 8:3 /data/late /data/late rw - ext4 /dev/sda rw\n\
              2 1 0:22 / /data/late rw - tmpfs tmpfs rw\n\
              1 0 8:3 / / rw - ext4 /dev/sda rw\n",
        );
        let mounts = platform::visible_mount_entries(&entries);
        assert!(!outside_boundary(
            Path::new("/data/late/child"),
            Some(0x803),
            Path::new("/data"),
            0x803,
            true,
            &mounts,
        ));
    }
}

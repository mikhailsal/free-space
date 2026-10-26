use super::*;

#[test]
fn getent_lines_parse_into_users() {
    let u = parse_passwd_line("ann:*:1500:1500:Ann,,,:/home/ann:/bin/bash\n").unwrap();
    assert_eq!((u.name.as_str(), u.uid, u.gid), ("ann", 1500, 1500));
    assert_eq!(u.home, PathBuf::from("/home/ann"));
    assert!(parse_passwd_line("").is_none());
    assert!(parse_passwd_line("ann:x:notanumber:1:::").is_none());
    // the uid form finds the same entry as the name form
    let me = user_by_uid(unsafe { libc::geteuid() }).unwrap();
    assert_eq!(getent_passwd(&me.name).map(|u| u.uid), Some(me.uid));
}

const SAMPLE: &str = "\
22 1 8:3 / / rw,relatime shared:1 - ext4 /dev/sda3 rw,errors=remount-ro
25 22 0:22 / /proc rw,nosuid shared:12 - proc proc rw
30 22 8:2 / /boot/efi rw,relatime shared:3 - vfat /dev/sda2 rw
31 22 7:1 / /snap/core/1 ro,nodev shared:4 - squashfs /dev/loop1 ro
40 22 8:3 /home/x /mnt/bind rw,relatime shared:1 - ext4 /dev/sda3 rw
41 22 8:5 / /mnt/my\\040disk rw - ext4 /dev/sda5 rw
";

#[test]
fn self_bind_mounts_are_walked_subtree_binds_are_not() {
    // inside a unit with ProtectSystem=strict and ReadWritePaths=/var/lib/freespace
    let unit = format!(
        "{SAMPLE}50 22 8:3 /home /home rw,relatime shared:1 - ext4 /dev/sda3 rw\n\
             51 22 8:3 /var/lib/freespace /var/lib/freespace rw shared:1 - ext4 /dev/sda3 rw\n"
    );
    let m = parse_mountinfo(&unit);
    assert_eq!(
        bind_mounts_below(&m, Path::new("/"), "8:3"),
        [PathBuf::from("/mnt/bind")]
    );
    // a root inside the self-bind: the same rule from that mount
    assert!(bind_mounts_below(&m, Path::new("/home/u"), "8:3").is_empty());
}

#[test]
fn descendants_attached_below_a_visible_mountpoint_are_hidden() {
    let entries = parse_mountinfo_entries(
        b"1 0 8:3 / / rw - ext4 disk rw\n\
              2 1 8:3 /elsewhere /data/late/child rw - ext4 disk rw\n\
              3 1 8:3 /data/late /data/late rw - ext4 disk rw\n",
    );
    let visible = visible_mount_entries(&entries);
    let ids: Vec<_> = visible.iter().map(|entry| entry.mount_id).collect();
    assert!(ids.contains(&3));
    assert!(!ids.contains(&2));
}

#[test]
fn top_of_stacked_mount_remains_visible_for_bind_boundaries() {
    let entries = parse_mountinfo_entries(
        b"1 0 8:3 / / rw - ext4 disk rw\n\
              2 1 0:22 / /data/late rw - tmpfs tmpfs rw\n\
              3 2 8:3 /elsewhere /data/late rw - ext4 disk rw\n",
    );
    let visible = visible_mount_entries(&entries);
    assert!(visible.iter().any(|entry| entry.mount_id == 3));
    let mounts: Vec<_> = visible.into_iter().map(|entry| entry.mount).collect();
    assert!(
        bind_mounts_below(&mounts, Path::new("/"), "8:3").contains(&PathBuf::from("/data/late"))
    );
}

#[test]
fn raw_mount_paths_preserve_the_whole_table_and_bind_boundaries() {
    let mut text = SAMPLE.as_bytes().to_vec();
    text.extend_from_slice(b"50 22 8:3 /other /raw/\xff rw - ext4 /dev/sda3 rw\n");
    text.extend_from_slice(b"51 22 8:3 /raw/self /raw/self rw - ext4 /dev/sda3 rw\n");
    text.extend_from_slice(br"52 22 8:3 /other\134name /raw/space\040tab\011line\012slash\134end rw - ext4 /dev/sda3 rw");
    // read_to_string would discard all mounts, including unrelated valid rows.
    assert!(std::str::from_utf8(&text).is_err());
    let mounts = parse_mountinfo_bytes(&text);
    assert_eq!(mounts.len(), 9);
    assert_eq!(mounts[0].mount_point, Path::new("/"));
    assert_eq!(mounts[6].mount_point.as_os_str().as_bytes(), b"/raw/\xff");
    assert_eq!(mounts[8].root.as_os_str().as_bytes(), br"/other\name");
    assert_eq!(
        mounts[8].mount_point.as_os_str().as_bytes(),
        b"/raw/space tab\tline\nslash\\end"
    );
    let raw = Path::new(OsStr::from_bytes(b"/raw/\xff"));
    assert_eq!(
        mount_for(&mounts, &raw.join("child")).unwrap().mount_point,
        raw
    );
    assert_eq!(
        mount_for(&mounts, Path::new(OsStr::from_bytes(b"/raw/\xffsibling")))
            .unwrap()
            .mount_point,
        Path::new("/")
    );
    let binds = bind_mounts_below(&mounts, Path::new("/raw"), "8:3");
    assert!(binds.contains(&raw.to_path_buf()));
    assert!(!binds.contains(&PathBuf::from("/raw/self")));
}

#[test]
fn parses_and_unescapes() {
    let m = parse_mountinfo(SAMPLE);
    assert_eq!(m.len(), 6);
    assert_eq!(m[5].mount_point, PathBuf::from("/mnt/my disk"));
    assert_eq!(m[4].root, PathBuf::from("/home/x"));
}

#[test]
fn default_roots_skip_pseudo_loop_and_bind() {
    let m = parse_mountinfo(SAMPLE);
    let r = default_roots(&m);
    assert_eq!(
        r,
        vec![
            PathBuf::from("/"),
            PathBuf::from("/boot/efi"),
            PathBuf::from("/mnt/my disk")
        ]
    );
}

#[test]
fn mounts_below_root() {
    let m = parse_mountinfo(SAMPLE);
    let below = mount_points_below(&m, Path::new("/"));
    assert!(below.contains(&PathBuf::from("/proc")));
    assert!(below.contains(&PathBuf::from("/mnt/bind")));
    assert!(!below.contains(&PathBuf::from("/")));
}

use super::*;
use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};

fn walk_dir(p: &Path, workers: usize) -> Tree {
    let mut o = WalkOptions::new(vec![p.to_path_buf()]);
    o.workers = workers;
    walk_with_mounts(&o, &[]).unwrap()
}

fn names_preorder(t: &mut Tree) -> Vec<String> {
    let f = t.finalize();
    f.order
        .iter()
        .map(|&r| String::from_utf8_lossy(t.name(&t.nodes[r as usize])).into_owned())
        .collect()
}

#[test]
fn walks_sorted_and_counts() {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir_all(d.path().join("b/c")).unwrap();
    fs::write(d.path().join("b/c/f1"), vec![1u8; 10_000]).unwrap();
    fs::write(d.path().join("a.txt"), b"hello").unwrap();
    symlink("/nonexistent", d.path().join("zlink")).unwrap();
    for w in [1, 4] {
        let mut t = walk_dir(d.path(), w);
        let names = names_preorder(&mut t);
        assert_eq!(&names[1..], &["a.txt", "b", "c", "f1", "zlink"]);
        let root = t.nodes[0];
        assert_eq!(root.items, 5);
        assert!(root.apparent >= 10_005);
        assert_eq!(root.errors, 0);
    }
}

#[test]
fn denied_dir_is_flagged() {
    if unsafe { libc::geteuid() } == 0 {
        return; // root reads everything
    }
    let d = tempfile::tempdir().unwrap();
    let locked = d.path().join("locked");
    fs::create_dir(&locked).unwrap();
    fs::write(locked.join("secret"), b"x").unwrap();
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
    let mut t = walk_dir(d.path(), 2);
    let f = t.finalize();
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(f.denied.len(), 1);
    assert_eq!(t.nodes[0].errors, 1);
}

#[test]
fn wide_fanout_under_low_nofile() {
    // T3: 20k-wide fan-out must not hit EMFILE; open fds stay at `workers`.
    let d = tempfile::tempdir().unwrap();
    for i in 0..20_000 {
        fs::create_dir(d.path().join(format!("d{i:05}"))).unwrap();
    }
    let mut t = walk_dir(d.path(), 8);
    t.finalize();
    assert_eq!(t.nodes[0].items, 20_000);
    assert_eq!(t.nodes[0].errors, 0);
}

#[test]
fn hardlinks_counted_once() {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir(d.path().join("x")).unwrap();
    fs::write(d.path().join("x/big"), vec![7u8; 1 << 20]).unwrap();
    fs::hard_link(d.path().join("x/big"), d.path().join("link")).unwrap();
    let mut t = walk_dir(d.path(), 3);
    let f = t.finalize();
    assert_eq!(f.hardlinks.len(), 2);
    let total = t.nodes[0].allocated;
    let x = t.nodes.iter().find(|n| t.name(n) == b"x").unwrap();
    // "/…/link" < "/…/x/big": the top-level link carries the size.
    assert!(x.allocated < 1 << 20);
    assert!((1 << 20..2 << 20).contains(&total));
}

#[test]
fn dev_string_matches_mountinfo_format() {
    assert_eq!(dev_string(0x803), "8:3");
    assert_eq!(dev_string(0x16), "0:22");
}

#[test]
fn hidden_stacked_bind_does_not_hide_visible_child_from_walk() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let late = root.join("late");
    let child = late.join("child");
    fs::create_dir_all(&child).unwrap();
    fs::write(child.join("visible"), b"visible data").unwrap();
    let dev = dev_string(fs::metadata(root).unwrap().dev());
    let text = format!(
        "1 0 {dev} / / rw - ext4 disk rw\n\
             2 1 0:22 / /{} rw - tmpfs tmpfs rw\n\
             4 2 {dev} /elsewhere /{} rw - ext4 disk rw\n\
             3 2 {dev} /{} /{} rw - ext4 disk rw\n",
        late.display(),
        child.display(),
        late.display(),
        late.display(),
    );
    let entries = platform::parse_mountinfo_entries(text.as_bytes());
    let raw_mounts: Vec<_> = entries.iter().map(|entry| entry.mount.clone()).collect();
    let visible_entries = platform::visible_mount_entries(&entries);
    let visible_mounts: Vec<_> = visible_entries
        .iter()
        .map(|entry| entry.mount.clone())
        .collect();

    assert!(platform::bind_mounts_below(&raw_mounts, root, &dev).contains(&child));
    assert!(!platform::bind_mounts_below(&visible_mounts, root, &dev).contains(&child));

    let mut opts = WalkOptions::new(vec![root.to_path_buf()]);
    opts.one_file_system = true;
    let mut tree = walk_with_mounts(&opts, &visible_mounts).unwrap();
    assert!(
        names_preorder(&mut tree)
            .iter()
            .any(|name| name == "visible")
    );
}

#[test]
fn descendant_bind_attached_below_visible_mountpoint_does_not_hide_files() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let late = root.join("late");
    let child = late.join("child");
    fs::create_dir_all(&child).unwrap();
    fs::write(child.join("visible"), b"visible data").unwrap();
    let dev = dev_string(fs::metadata(root).unwrap().dev());
    let text = format!(
        "1 0 {dev} / / rw - ext4 disk rw\n\
             2 1 {dev} /elsewhere /{} rw - ext4 disk rw\n\
             3 1 {dev} /{} /{} rw - ext4 disk rw\n",
        child.display(),
        late.display(),
        late.display(),
    );
    let entries = platform::parse_mountinfo_entries(text.as_bytes());
    let raw_mounts: Vec<_> = entries.iter().map(|entry| entry.mount.clone()).collect();
    let visible_entries = platform::visible_mount_entries(&entries);
    let visible_mounts: Vec<_> = visible_entries
        .iter()
        .map(|entry| entry.mount.clone())
        .collect();

    assert!(platform::bind_mounts_below(&raw_mounts, root, &dev).contains(&child));
    assert!(!platform::bind_mounts_below(&visible_mounts, root, &dev).contains(&child));

    let mut opts = WalkOptions::new(vec![root.to_path_buf()]);
    opts.one_file_system = true;
    let mut tree = walk_with_mounts(&opts, &visible_mounts).unwrap();
    assert!(
        names_preorder(&mut tree)
            .iter()
            .any(|name| name == "visible")
    );
}

#[test]
fn top_stacked_subtree_bind_remains_a_walk_boundary() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let late = root.join("late");
    fs::create_dir_all(&late).unwrap();
    fs::write(late.join("hidden_by_bind"), b"must not be walked").unwrap();
    let dev = dev_string(fs::metadata(root).unwrap().dev());
    let text = format!(
        "1 0 {dev} / / rw - ext4 disk rw\n\
             2 1 0:22 / /{} rw - tmpfs tmpfs rw\n\
             3 2 {dev} /elsewhere /{} rw - ext4 disk rw\n",
        late.display(),
        late.display(),
    );
    let entries = platform::parse_mountinfo_entries(text.as_bytes());
    let visible_entries = platform::visible_mount_entries(&entries);
    let visible_mounts: Vec<_> = visible_entries
        .iter()
        .map(|entry| entry.mount.clone())
        .collect();
    assert!(visible_entries.iter().any(|entry| entry.mount_id == 3));
    assert!(platform::bind_mounts_below(&visible_mounts, root, &dev).contains(&late));

    let mut opts = WalkOptions::new(vec![root.to_path_buf()]);
    opts.one_file_system = true;
    let mut tree = walk_with_mounts(&opts, &visible_mounts).unwrap();
    assert!(
        !names_preorder(&mut tree)
            .iter()
            .any(|name| name == "hidden_by_bind")
    );
}

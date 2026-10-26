use freespace_core::{
    ErrorCode,
    job::{ScanRequest, run_scan},
    queries::{
        self,
        params::{DiffParams, FindParams, LsParams, TopParams},
    },
    refresh::{RefreshRequest, run_refresh},
    snapshot::{
        SnapshotDir,
        reader::Snapshot,
        v2::{Reader, Store},
    },
};
use serde::Serialize;
use serde_json::Value;
use std::{
    ffi::CString,
    fs,
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
    process::Command,
};

#[test]
fn live_mount_namespace_parity() {
    if std::env::var_os("FREESPACE_LIFECYCLE_MOUNT_CHILD").is_some() {
        isolated_mount_cases();
        return;
    }
    let namespace = fs::read_link("/proc/self/ns/mnt").unwrap();
    let probe = Command::new("sudo")
        .args(["-n", "unshare", "--mount", "--fork", "/bin/true"])
        .output();
    if !probe.is_ok_and(|out| out.status.success()) {
        eprintln!("live mount parity skipped: passwordless sudo and mount namespaces unavailable");
        return;
    }
    let out = Command::new("sudo")
        .args([
            "-n",
            "unshare",
            "--mount",
            "--fork",
            "env",
            "FREESPACE_LIFECYCLE_MOUNT_CHILD=1",
        ])
        .arg(format!(
            "FREESPACE_LIFECYCLE_PARENT_MNT={}",
            namespace.display()
        ))
        .arg(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "mounts::live_mount_namespace_parity",
            "--nocapture",
        ])
        .output()
        .unwrap();
    assert_eq!(fs::read_link("/proc/self/ns/mnt").unwrap(), namespace);
    assert!(
        out.status.success(),
        "mount child failed:\n{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stderr).contains("private mount propagation verified"));
    eprintln!("{}", String::from_utf8_lossy(&out.stderr));
}

fn isolate_propagation() {
    assert_eq!(unsafe { libc::geteuid() }, 0);
    let parent = std::env::var("FREESPACE_LIFECYCLE_PARENT_MNT").unwrap();
    assert_ne!(
        fs::read_link("/proc/self/ns/mnt")
            .unwrap()
            .to_string_lossy(),
        parent
    );
    let slash = CString::new("/").unwrap();
    assert_eq!(
        unsafe {
            libc::mount(
                std::ptr::null(),
                slash.as_ptr(),
                std::ptr::null(),
                libc::MS_REC | libc::MS_PRIVATE,
                std::ptr::null(),
            )
        },
        0
    );
    let info = fs::read_to_string("/proc/self/mountinfo").unwrap();
    for line in info.lines() {
        let fields = line.split(" - ").next().unwrap();
        assert!(
            !fields
                .split_whitespace()
                .any(|f| f.starts_with("shared:") || f.starts_with("master:")),
            "propagating mount remains: {line}"
        );
    }
    eprintln!("private mount propagation verified before fixture mounts");
}

#[derive(Default)]
struct Mounts(Vec<PathBuf>);
impl Mounts {
    fn add(&mut self, source: &Path, target: &Path, bind: bool) {
        fs::create_dir_all(target).unwrap();
        let src = CString::new(source.as_os_str().as_bytes()).unwrap();
        let dst = CString::new(target.as_os_str().as_bytes()).unwrap();
        let kind = CString::new("tmpfs").unwrap();
        let options = CString::new("size=8m,nr_inodes=256,mode=0755").unwrap();
        let flags = if bind { libc::MS_BIND } else { 0 };
        let rc = unsafe {
            libc::mount(
                src.as_ptr(),
                dst.as_ptr(),
                if bind {
                    std::ptr::null()
                } else {
                    kind.as_ptr()
                },
                flags,
                if bind {
                    std::ptr::null()
                } else {
                    options.as_ptr().cast()
                },
            )
        };
        assert_eq!(
            rc,
            0,
            "fixture mount {}: {}",
            target.display(),
            std::io::Error::last_os_error()
        );
        self.0.push(target.into());
    }
    fn pop(&mut self) {
        let target = self.0.pop().unwrap();
        let path = CString::new(target.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::umount2(path.as_ptr(), 0) }, 0);
    }
}
impl Drop for Mounts {
    fn drop(&mut self) {
        for target in self.0.iter().rev() {
            let path = CString::new(target.as_os_str().as_bytes()).unwrap();
            unsafe {
                libc::umount2(path.as_ptr(), libc::MNT_DETACH);
            }
        }
    }
}

struct Fixture {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    db: PathBuf,
    v1: SnapshotDir,
    store: Store,
}
impl Fixture {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("tree");
        for dir in ["local", "disk", "bind", "late"] {
            fs::create_dir_all(root.join(dir)).unwrap();
        }
        fs::write(root.join("local/a"), vec![1; 4096]).unwrap();
        fs::write(root.join("late/old"), [1]).unwrap();
        let db = tmp.path().join("v2.db");
        let v1 = SnapshotDir::new(tmp.path().join("v1"));
        let store = Store::open(&db).unwrap();
        Self {
            _tmp: tmp,
            root,
            db,
            v1,
            store,
        }
    }
    fn scan(&mut self, roots: &[PathBuf], one_fs: bool, disk: bool) {
        run_scan(ScanRequest {
            roots: roots.to_vec(),
            one_file_system: one_fs,
            workers: 1,
            low_memory: disk,
            dir: self.v1.clone(),
            keep_previous: true,
            on_progress: None,
        })
        .unwrap();
        self.store
            .scan_roots_with_options(roots, one_fs, 1, disk, true)
            .unwrap();
        self.parity(roots);
    }
    fn refresh(&mut self, path: &Path, roots: &[PathBuf]) {
        run_refresh(RefreshRequest {
            path: path.as_os_str().as_bytes().to_vec(),
            workers: 1,
            dir: self.v1.clone(),
            on_progress: None,
        })
        .unwrap();
        self.store.refresh(path, 1).unwrap();
        self.parity(roots);
    }
    fn kind(&self, relative: &str) -> freespace_core::scan::Kind {
        Reader::open(&self.db)
            .unwrap()
            .get(self.root.join(relative).as_os_str().as_bytes(), false)
            .unwrap()
            .unwrap()
            .kind
    }
    fn refused(&mut self, path: &Path) {
        let before = Reader::open(&self.db).unwrap().generations;
        let old = Snapshot::open_current(&self.v1).unwrap().meta.generation;
        let v1 = run_refresh(RefreshRequest {
            path: path.as_os_str().as_bytes().to_vec(),
            workers: 1,
            dir: self.v1.clone(),
            on_progress: None,
        })
        .unwrap_err();
        let v2 = self.store.refresh(path, 1).unwrap_err();
        assert_eq!((v1.code, v2.code), (ErrorCode::Usage, ErrorCode::Usage));
        assert_eq!(Reader::open(&self.db).unwrap().generations, before);
        assert_eq!(
            Snapshot::open_current(&self.v1).unwrap().meta.generation,
            old
        );
    }
    fn parity(&self, roots: &[PathBuf]) {
        let old = Snapshot::open_current(&self.v1).unwrap();
        let reader = Reader::open(&self.db).unwrap();
        for root in roots {
            let path = Some(root.display().to_string());
            let ls = LsParams {
                path: path.clone(),
                limit: Some(1000),
                ..Default::default()
            };
            same(
                queries::ls(&old, &ls).unwrap(),
                queries::v2::ls(&reader, &ls).unwrap(),
            );
            for kind in ["file", "dir"] {
                let top = TopParams {
                    path: path.clone(),
                    kind: Some(kind.into()),
                    ..Default::default()
                };
                same(
                    queries::top(&old, &top).unwrap(),
                    queries::v2::top(&reader, &top).unwrap(),
                );
            }
            let find = FindParams {
                path,
                kind: Some("any".into()),
                limit: Some(1000),
                ..Default::default()
            };
            same(
                queries::find(&old, &find).unwrap(),
                queries::v2::find(&reader, &find).unwrap(),
            );
            let diff = DiffParams {
                path: Some(root.display().to_string()),
                depth: Some(2),
                kind: Some("any".into()),
                limit: Some(1000),
                ..Default::default()
            };
            match (
                queries::diff(&self.v1, &diff),
                queries::v2::diff(&reader, &diff),
            ) {
                (Ok(old), Ok(new)) => same(old, new),
                (Err(old), Err(new)) => assert_eq!(old.code, new.code),
                _ => panic!("v1/v2 diff success boundary differs"),
            }
        }
        same(
            queries::status(&self.v1).unwrap(),
            queries::v2::status(&reader, &self.v1).unwrap(),
        );
    }
}

fn same<T: Serialize>(old: T, new: T) {
    assert_eq!(comparable(old), comparable(new));
}
fn comparable(value: impl Serialize) -> Value {
    fn strip(value: &mut Value) {
        match value {
            Value::Object(map) => {
                // Independent scans differ in identity/time/live free-space.
                // Preserve all allocated/apparent/items/errors and filter totals.
                for key in [
                    "snapshot_id",
                    "schema_version",
                    "scanned_at",
                    "prev_scanned_at",
                    "age_seconds",
                    "used",
                    "avail",
                    "fs_used",
                    "fs_avail",
                ] {
                    map.remove(key);
                }
                for value in map.values_mut() {
                    strip(value);
                }
            }
            Value::Array(values) => values.iter_mut().for_each(strip),
            _ => {}
        }
    }
    let mut value = serde_json::to_value(value).unwrap();
    strip(&mut value);
    value
}

fn isolated_mount_cases() {
    isolate_propagation();
    let mut f = Fixture::new();
    // Mounts drop before the fixture, so root's temporary directory is cleaned.
    let mut mounts = Mounts::default();
    let source = f._tmp.path().join("bind-source");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("bound-file"), vec![2; 8192]).unwrap();
    mounts.add(Path::new("tmpfs"), &f.root.join("disk"), false);
    mounts.add(&source, &f.root.join("bind"), true);
    fs::create_dir(f.root.join("disk/sub")).unwrap();
    fs::write(f.root.join("disk/sub/data"), vec![3; 16384]).unwrap();
    let roots = vec![f.root.clone()];
    for disk in [false, true] {
        f.scan(&roots, true, disk);
        assert_eq!(f.kind("disk"), freespace_core::scan::Kind::Mount);
        assert_eq!(f.kind("bind"), freespace_core::scan::Kind::Mount);
        f.refused(&f.root.join("disk"));
        f.refused(&f.root.join("bind"));
        fs::write(f.root.join("local/new"), vec![4; 8192]).unwrap();
        f.refresh(&f.root.join("local"), &roots);
    }
    // A real late mount must be refused before it enters the recorded tree.
    mounts.add(Path::new("tmpfs"), &f.root.join("late"), false);
    f.refused(&f.root.join("late"));
    mounts.pop();
    for disk in [false, true] {
        f.scan(&roots, false, disk);
        assert_eq!(f.kind("disk"), freespace_core::scan::Kind::Dir);
        assert_eq!(f.kind("bind"), freespace_core::scan::Kind::Mount);
        f.refused(&f.root.join("bind"));
        fs::write(f.root.join("disk/sub/new"), [5]).unwrap();
        f.refresh(&f.root.join("disk/sub"), &roots);
    }
    let nested = vec![f.root.clone(), f.root.join("disk")];
    for disk in [false, true] {
        f.scan(&nested, true, disk);
        assert_eq!(f.kind("disk"), freespace_core::scan::Kind::Dir);
        fs::write(f.root.join("disk/sub/explicit"), [6]).unwrap();
        f.refresh(&f.root.join("disk"), &nested);
        f.refused(&f.root.join("bind"));
    }
    eprintln!(
        "live tmpfs + same-device bind parity: one filesystem, cross mounts, nested roots, refreshes, late-mount/bind refusals; memory + disk profiles passed"
    );
}

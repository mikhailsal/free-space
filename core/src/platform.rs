//! Platform hooks (plan: "Платформенные крючки"): allocated size, file identity,
//! mount boundaries, filesystem stats. v1 ships the Linux implementation only.

use std::collections::HashSet;
use std::ffi::{CString, OsStr};
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

pub mod memory;

/// statvfs numbers for one filesystem.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FsStats {
    pub total: u64,
    pub free: u64,
    /// Space available to unprivileged users (what `df` calls "Avail").
    pub avail: u64,
    pub used: u64,
    pub inodes_total: u64,
    pub inodes_free: u64,
}

impl FsStats {
    pub fn inodes_used(&self) -> Option<u64> {
        if self.inodes_total == 0 {
            None
        } else {
            Some(self.inodes_total.saturating_sub(self.inodes_free))
        }
    }
}

pub fn statvfs(path: &Path) -> io::Result<FsStats> {
    let c = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path contains NUL"))?;
    let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut s) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let frsize = if s.f_frsize > 0 {
        s.f_frsize
    } else {
        s.f_bsize
    } as u64;
    let total = s.f_blocks as u64 * frsize;
    let free = s.f_bfree as u64 * frsize;
    Ok(FsStats {
        total,
        free,
        avail: s.f_bavail as u64 * frsize,
        used: total.saturating_sub(free),
        inodes_total: s.f_files as u64,
        inodes_free: s.f_ffree as u64,
    })
}

/// Raise the soft `RLIMIT_NOFILE` to the hard limit (T3). Returns the new soft limit.
pub fn raise_nofile_limit() -> u64 {
    let mut r = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    unsafe {
        if libc::getrlimit(libc::RLIMIT_NOFILE, &mut r) != 0 {
            return 256;
        }
        if r.rlim_cur < r.rlim_max {
            let want = libc::rlimit {
                rlim_cur: r.rlim_max,
                rlim_max: r.rlim_max,
            };
            if libc::setrlimit(libc::RLIMIT_NOFILE, &want) == 0 {
                r.rlim_cur = r.rlim_max;
            }
        }
    }
    r.rlim_cur
}

pub fn hostname() -> String {
    let mut buf = [0u8; 256];
    let rc = unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) };
    if rc != 0 {
        return "unknown".into();
    }
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..end]).into_owned()
}

pub fn os_name() -> String {
    std::env::consts::OS.to_string()
}

pub fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// One line of /proc/self/mountinfo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MountInfo {
    pub mount_point: PathBuf,
    pub fs_type: String,
    pub source: String,
    /// "major:minor"
    pub dev: String,
    /// Root of the mount within its filesystem; "/" unless it is a bind mount of a subtree.
    pub root: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MountInfoEntry {
    pub mount_id: u64,
    pub parent_id: u64,
    pub mount: MountInfo,
}

/// Decode the octal escapes (`\040` for space etc.) used by the kernel in mountinfo.
fn unescape_mount(b: &[u8]) -> PathBuf {
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\'
            && i + 3 < b.len()
            && b[i + 1..i + 4].iter().all(|c| (b'0'..=b'7').contains(c))
        {
            let v = (b[i + 1] - b'0') * 64 + (b[i + 2] - b'0') * 8 + (b[i + 3] - b'0');
            out.push(v);
            i += 4;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    PathBuf::from(OsStr::from_bytes(&out))
}

pub fn parse_mountinfo(text: &str) -> Vec<MountInfo> {
    parse_mountinfo_bytes(text.as_bytes())
}

fn parse_mountinfo_bytes(text: &[u8]) -> Vec<MountInfo> {
    parse_mountinfo_entries(text)
        .into_iter()
        .map(|entry| entry.mount)
        .collect()
}

pub(crate) fn parse_mountinfo_entries(text: &[u8]) -> Vec<MountInfoEntry> {
    let mut v = Vec::new();
    for line in text.split(|b| *b == b'\n') {
        let fields: Vec<&[u8]> = line.split(|b| *b == b' ').collect();
        let Some(sep) = fields.iter().position(|f| *f == b"-") else {
            continue;
        };
        if fields.len() < 5 || fields.len() < sep + 3 {
            continue;
        }
        let (Ok(mount_id), Ok(parent_id)) = (
            String::from_utf8_lossy(fields[0]).parse(),
            String::from_utf8_lossy(fields[1]).parse(),
        ) else {
            continue;
        };
        v.push(MountInfoEntry {
            mount_id,
            parent_id,
            mount: MountInfo {
                dev: String::from_utf8_lossy(fields[2]).into_owned(),
                root: unescape_mount(fields[3]),
                mount_point: unescape_mount(fields[4]),
                fs_type: String::from_utf8_lossy(fields[sep + 1]).into_owned(),
                source: String::from_utf8_lossy(fields[sep + 2]).into_owned(),
            },
        });
    }
    v
}

pub fn mountinfo() -> Vec<MountInfo> {
    mountinfo_entries()
        .into_iter()
        .map(|entry| entry.mount)
        .collect()
}

pub(crate) fn mountinfo_entries() -> Vec<MountInfoEntry> {
    std::fs::read("/proc/self/mountinfo")
        .map(|text| parse_mountinfo_entries(&text))
        .unwrap_or_default()
}

pub(crate) fn visible_mountinfo_entries() -> Vec<MountInfoEntry> {
    visible_mount_entries(&mountinfo_entries())
}

pub(crate) fn visible_mountinfo() -> Vec<MountInfo> {
    visible_mountinfo_entries()
        .into_iter()
        .map(|entry| entry.mount)
        .collect()
}

pub(crate) fn visible_mount_entries(mounts: &[MountInfoEntry]) -> Vec<MountInfoEntry> {
    let roots: Vec<&MountInfoEntry> = mounts
        .iter()
        .filter(|entry| entry.mount.mount_point == Path::new("/"))
        .collect();
    let mut top_roots = roots.iter().copied().filter(|entry| {
        !roots
            .iter()
            .any(|child| child.mount_id != entry.mount_id && child.parent_id == entry.mount_id)
    });
    let Some(root) = top_roots.next() else {
        return Vec::new();
    };
    if top_roots.next().is_some() {
        return Vec::new();
    }

    let mut visible = Vec::new();
    let mut pending = vec![root.clone()];
    let mut visited = HashSet::new();
    while let Some(parent) = pending.pop() {
        if !visited.insert(parent.mount_id) {
            continue;
        }
        visible.push(parent.clone());
        let mut points = child_mount_points(mounts, parent.mount_id);
        // Process ancestor mountpoints before descendants. A mount attached to
        // this parent below a visible mountpoint is hidden unless that visible
        // mount is its actual parent in mountinfo's ID topology.
        points.sort_by_key(|point| point.components().count());
        for point in points {
            queue_visible_mount(mounts, &parent, &point, &visible, &mut pending);
        }
    }
    visible
}

fn child_mount_points(mounts: &[MountInfoEntry], parent_id: u64) -> Vec<PathBuf> {
    let mut points = Vec::new();
    for child in mounts
        .iter()
        .filter(|entry| entry.parent_id == parent_id && entry.mount_id != parent_id)
    {
        if !points.contains(&child.mount.mount_point) {
            points.push(child.mount.mount_point.clone());
        }
    }
    points
}

fn queue_visible_mount(
    mounts: &[MountInfoEntry],
    parent: &MountInfoEntry,
    point: &Path,
    visible: &[MountInfoEntry],
    pending: &mut Vec<MountInfoEntry>,
) {
    let Some(top) = top_mount_at(mounts, parent.mount_id, point) else {
        return;
    };
    let covering = visible
        .iter()
        .chain(pending.iter())
        .filter(|entry| {
            entry.mount.mount_point != point && point.starts_with(&entry.mount.mount_point)
        })
        .max_by_key(|entry| entry.mount.mount_point.components().count());
    if covering.is_none_or(|entry| entry.mount_id == parent.mount_id) {
        pending.push(top);
    }
}

fn top_mount_at(
    mounts: &[MountInfoEntry],
    parent_id: u64,
    mount_point: &Path,
) -> Option<MountInfoEntry> {
    let mut current = unique_mount_child(mounts, parent_id, mount_point)?;
    while let Some(top) = unique_mount_child(mounts, current.mount_id, mount_point) {
        current = top;
    }
    Some(current)
}

fn unique_mount_child(
    mounts: &[MountInfoEntry],
    parent_id: u64,
    mount_point: &Path,
) -> Option<MountInfoEntry> {
    let mut matches = mounts
        .iter()
        .filter(|entry| entry.parent_id == parent_id && entry.mount.mount_point == mount_point);
    let child = matches.next()?.clone();
    matches.next().is_none().then_some(child)
}

/// Filesystems `install-timer` puts into the default root set.
pub const REAL_FS: &[&str] = &[
    "ext2", "ext3", "ext4", "xfs", "btrfs", "ntfs", "ntfs3", "fuseblk", "exfat", "vfat", "f2fs",
    "zfs",
];

/// Default root set: real filesystems from mountinfo, one per device, skipping
/// snap loop mounts and bind mounts of subtrees (plan: "Корни сканирования").
pub fn default_roots(mounts: &[MountInfo]) -> Vec<PathBuf> {
    let mut seen_dev = std::collections::HashSet::new();
    let mut roots = Vec::new();
    let mut sorted: Vec<&MountInfo> = mounts.iter().collect();
    sorted.sort_by_key(|m| m.mount_point.as_os_str().len());
    for m in sorted {
        if !REAL_FS.contains(&m.fs_type.as_str()) {
            continue;
        }
        if m.source.starts_with("/dev/loop") || m.mount_point.starts_with("/snap") {
            continue;
        }
        if m.root != Path::new("/") && m.fs_type != "btrfs" {
            continue; // bind mount of a subtree
        }
        let key = if m.fs_type == "btrfs" {
            format!("{}:{}", m.dev, m.root.display())
        } else {
            m.dev.clone()
        };
        if seen_dev.insert(key) {
            roots.push(m.mount_point.clone());
        }
    }
    roots
}

/// Mount points strictly below `root` (where the walk must stop with one-file-system),
/// plus bind mounts of the same device, which `st_dev` alone cannot see.
pub fn mount_points_below(mounts: &[MountInfo], root: &Path) -> Vec<PathBuf> {
    mounts
        .iter()
        .filter(|m| m.mount_point != root && m.mount_point.starts_with(root))
        .map(|m| m.mount_point.clone())
        .collect()
}

/// Mounts of `dev` strictly below `root` that show another part of the
/// filesystem (bind mounts of a subtree): the walk stops there so nothing is
/// counted twice, and `st_dev` alone cannot see them. A directory mounted onto
/// itself shows its own content and is walked like any directory: systemd's
/// `ProtectSystem=strict` mounts /home, /root and every `ReadWritePaths` that
/// way inside the unit.
pub fn bind_mounts_below(mounts: &[MountInfo], root: &Path, dev: &str) -> Vec<PathBuf> {
    let base = mount_for(mounts, root);
    mounts
        .iter()
        .filter(|m| m.mount_point != root && m.mount_point.starts_with(root) && m.dev == dev)
        .filter(|m| {
            let own = base.as_ref().and_then(|b| {
                let rel = m.mount_point.strip_prefix(&b.mount_point).ok()?;
                Some(b.root.join(rel))
            });
            own.as_deref() != Some(m.root.as_path())
        })
        .map(|m| m.mount_point.clone())
        .collect()
}

pub fn mount_for(mounts: &[MountInfo], path: &Path) -> Option<MountInfo> {
    mounts
        .iter()
        .filter(|m| path.starts_with(&m.mount_point))
        .max_by_key(|m| m.mount_point.as_os_str().len())
        .cloned()
}

#[cfg(test)]
mod tests;

/// A passwd entry: what `install-timer` and `skill install` need to act for
/// `$SUDO_USER` (ACL, home directory, file ownership).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserInfo {
    pub name: String,
    pub uid: u32,
    pub gid: u32,
    pub home: PathBuf,
}

fn user_from(pw: *const libc::passwd) -> Option<UserInfo> {
    if pw.is_null() {
        return None;
    }
    let pw = unsafe { &*pw };
    let s = |p: *const libc::c_char| {
        unsafe { std::ffi::CStr::from_ptr(p) }
            .to_string_lossy()
            .into_owned()
    };
    Some(UserInfo {
        name: s(pw.pw_name),
        uid: pw.pw_uid,
        gid: pw.pw_gid,
        home: PathBuf::from(s(pw.pw_dir)),
    })
}

pub fn user_by_name(name: &str) -> Option<UserInfo> {
    let c = std::ffi::CString::new(name).ok()?;
    user_from(unsafe { libc::getpwnam(c.as_ptr()) }).or_else(|| getent_passwd(name))
}

pub fn user_by_uid(uid: u32) -> Option<UserInfo> {
    user_from(unsafe { libc::getpwuid(uid) }).or_else(|| getent_passwd(&uid.to_string()))
}

/// The release binary is static musl, whose getpwnam reads /etc/passwd only;
/// LDAP or SSSD users come from the host's own NSS through getent(1).
fn getent_passwd(key: &str) -> Option<UserInfo> {
    let o = std::process::Command::new("getent")
        .args(["passwd", key])
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if !o.status.success() {
        return None;
    }
    parse_passwd_line(&String::from_utf8_lossy(&o.stdout))
}

/// `name:x:uid:gid:gecos:home:shell`
fn parse_passwd_line(line: &str) -> Option<UserInfo> {
    let f: Vec<&str> = line.lines().next()?.split(':').collect();
    if f.len() < 7 || f[0].is_empty() {
        return None;
    }
    Some(UserInfo {
        name: f[0].to_owned(),
        uid: f[2].parse().ok()?,
        gid: f[3].parse().ok()?,
        home: PathBuf::from(f[5]),
    })
}

/// The human behind this process: `$SUDO_USER` under sudo, else the caller.
pub fn invoking_user() -> Option<UserInfo> {
    match std::env::var("SUDO_USER") {
        Ok(n) if !n.is_empty() && n != "root" => user_by_name(&n),
        _ => {
            let uid = unsafe { libc::geteuid() };
            if uid == 0 { None } else { user_by_uid(uid) }
        }
    }
}

/// The passwd entry of the effective user.
pub fn current_user() -> Option<UserInfo> {
    user_by_uid(unsafe { libc::geteuid() })
}

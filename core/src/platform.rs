//! Platform hooks (plan: "Платформенные крючки"): allocated size, file identity,
//! mount boundaries, filesystem stats. v1 ships the Linux implementation only.

use std::ffi::{CString, OsStr};
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

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

/// Decode the octal escapes (`\040` for space etc.) used by the kernel in mountinfo.
fn unescape_mount(s: &str) -> PathBuf {
    let b = s.as_bytes();
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
    let mut v = Vec::new();
    for line in text.lines() {
        let fields: Vec<&str> = line.split(' ').collect();
        let Some(sep) = fields.iter().position(|f| *f == "-") else {
            continue;
        };
        if fields.len() < 5 || fields.len() < sep + 3 {
            continue;
        }
        v.push(MountInfo {
            dev: fields[2].to_string(),
            root: unescape_mount(fields[3]),
            mount_point: unescape_mount(fields[4]),
            fs_type: fields[sep + 1].to_string(),
            source: fields[sep + 2].to_string(),
        });
    }
    v
}

pub fn mountinfo() -> Vec<MountInfo> {
    std::fs::read_to_string("/proc/self/mountinfo")
        .map(|t| parse_mountinfo(&t))
        .unwrap_or_default()
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

pub fn mount_for(mounts: &[MountInfo], path: &Path) -> Option<MountInfo> {
    mounts
        .iter()
        .filter(|m| path.starts_with(&m.mount_point))
        .max_by_key(|m| m.mount_point.as_os_str().len())
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
22 1 8:3 / / rw,relatime shared:1 - ext4 /dev/sda3 rw,errors=remount-ro
25 22 0:22 / /proc rw,nosuid shared:12 - proc proc rw
30 22 8:2 / /boot/efi rw,relatime shared:3 - vfat /dev/sda2 rw
31 22 7:1 / /snap/core/1 ro,nodev shared:4 - squashfs /dev/loop1 ro
40 22 8:3 /home/x /mnt/bind rw,relatime shared:1 - ext4 /dev/sda3 rw
41 22 8:5 / /mnt/my\\040disk rw - ext4 /dev/sda5 rw
";

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
}

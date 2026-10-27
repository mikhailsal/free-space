//! Conservative Linux memory headroom, including visible cgroup ancestors.
//!
//! This is a point-in-time estimate, not a reservation. Cgroup usage includes
//! reclaimable cache, so subtracting it deliberately favors disk staging.
use std::io;
use std::path::{Component, Path, PathBuf};

/// Memory available to a new operation, bounded by host and visible cgroup caps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryAvailability {
    pub available_bytes: Option<u64>,
    /// A finite cap, or uncertainty about whether a cap applies, was observed.
    pub limited: bool,
}

/// Read Linux procfs/cgroup files without changing resource limits or caches.
pub fn memory_availability() -> MemoryAvailability {
    probe(|path| std::fs::read_to_string(path))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Version {
    V1,
    V2,
}

struct Mount {
    point: PathBuf,
    root: PathBuf,
}

enum Headroom {
    Unlimited,
    Finite(u64),
    Unknown,
}

fn probe(mut read: impl FnMut(&Path) -> io::Result<String>) -> MemoryAvailability {
    let host = read(Path::new("/proc/meminfo"))
        .ok()
        .and_then(|text| host_available(&text));
    let unknown = MemoryAvailability {
        available_bytes: None,
        limited: true,
    };
    let Ok(membership) = read(Path::new("/proc/self/cgroup")) else {
        return unknown;
    };
    let Ok(group) = membership_path(&membership) else {
        return unknown;
    };
    let Some((version, group)) = group else {
        return MemoryAvailability {
            available_bytes: host,
            limited: false,
        };
    };
    let Ok(mountinfo) = read(Path::new("/proc/self/mountinfo")) else {
        return unknown;
    };
    let Some((mount, mut current)) = resolve_mount(&mountinfo, version, &group) else {
        return unknown;
    };
    let mut result = MemoryAvailability {
        available_bytes: host,
        limited: false,
    };
    loop {
        let global_root = current == mount.point && mount.root == Path::new("/");
        match headroom(&mut read, &current, version, global_root) {
            Headroom::Unlimited => {}
            Headroom::Finite(bytes) => {
                result.limited = true;
                result.available_bytes = result.available_bytes.map(|host| host.min(bytes));
            }
            Headroom::Unknown => return unknown,
        }
        if current == mount.point {
            return result;
        }
        if !current.pop() || !current.starts_with(&mount.point) {
            return unknown;
        }
    }
}

fn host_available(text: &str) -> Option<u64> {
    let field = |name: &str| {
        let line = text.lines().find(|line| line.starts_with(name))?;
        let mut fields = line.strip_prefix(name)?.split_whitespace();
        let bytes = fields.next()?.parse::<u64>().ok()?.checked_mul(1024)?;
        (fields.next()? == "kB" && fields.next().is_none()).then_some(bytes)
    };
    Some(field("MemAvailable:")?.min(field("MemTotal:")?))
}

fn safe_absolute(path: &Path) -> bool {
    path.is_absolute()
        && !path.as_os_str().as_encoded_bytes().contains(&0)
        && path
            .components()
            .all(|part| matches!(part, Component::RootDir | Component::Normal(_)))
}

fn membership_path(text: &str) -> Result<Option<(Version, PathBuf)>, ()> {
    let mut unified = None;
    let mut memory = None;
    for line in text.lines().filter(|line| !line.is_empty()) {
        let mut fields = line.splitn(3, ':');
        let id = fields.next().ok_or(())?;
        let controllers = fields.next().ok_or(())?;
        let path = fields.next().ok_or(())?;
        let slot = if id == "0" && controllers.is_empty() {
            &mut unified
        } else if controllers.split(',').any(|part| part == "memory") {
            &mut memory
        } else {
            continue;
        };
        if slot.is_some() || !safe_absolute(Path::new(path)) {
            return Err(());
        }
        *slot = Some(PathBuf::from(path));
    }
    Ok(memory
        .map(|path| (Version::V1, path))
        .or_else(|| unified.map(|path| (Version::V2, path))))
}

fn resolve_mount(text: &str, version: Version, group: &Path) -> Option<(Mount, PathBuf)> {
    let entries = super::parse_mountinfo_entries(text.as_bytes());
    let visible = super::visible_mount_entries(&entries);
    let mut mounts = Vec::new();
    for entry in visible {
        let mount = entry.mount;
        let matching = match version {
            Version::V2 => mount.fs_type == "cgroup2",
            Version::V1 => mount.fs_type == "cgroup" && memory_mount(text, entry.mount_id),
        };
        if matching && safe_absolute(&mount.root) && safe_absolute(&mount.mount_point) {
            mounts.push(Mount {
                point: mount.mount_point,
                root: mount.root,
            });
        }
    }
    // Prefer the broadest visible mount: a subtree bind must not hide an
    // ancestor cap available through another full-hierarchy mount.
    mounts.sort_by_key(|mount| mount.root.components().count());
    if let Some(index) = mounts
        .iter()
        .position(|mount| group.starts_with(&mount.root))
    {
        let mount = mounts.remove(index);
        let current = mount.point.join(group.strip_prefix(&mount.root).ok()?);
        return Some((mount, current));
    }
    // A private cgroup namespace may report membership "/" while its existing
    // mount still exposes the global subtree root. Never append that global
    // prefix twice, and never escape the visible mount to inspect hidden parents.
    if group == Path::new("/") {
        let mount = mounts.into_iter().next()?;
        let current = mount.point.clone();
        return Some((mount, current));
    }
    None
}

fn memory_mount(text: &str, id: u64) -> bool {
    text.lines().any(|line| {
        let fields: Vec<_> = line.split_whitespace().collect();
        if fields.first().and_then(|value| value.parse::<u64>().ok()) != Some(id) {
            return false;
        }
        let Some(separator) = fields.iter().position(|field| *field == "-") else {
            return false;
        };
        fields
            .get(separator + 3)
            .is_some_and(|options| options.split(',').any(|option| option == "memory"))
    })
}

fn headroom(
    read: &mut impl FnMut(&Path) -> io::Result<String>,
    group: &Path,
    version: Version,
    global_root: bool,
) -> Headroom {
    let (limit_name, usage_name) = match version {
        Version::V1 => ("memory.limit_in_bytes", "memory.usage_in_bytes"),
        Version::V2 => ("memory.max", "memory.current"),
    };
    let limit = match read(&group.join(limit_name)) {
        Ok(limit) => limit,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return missing_controller(read, group, version, global_root, usage_name);
        }
        Err(_) => return Headroom::Unknown,
    };
    if version == Version::V2 && limit.trim() == "max" {
        return Headroom::Unlimited;
    }
    let Ok(limit) = limit.trim().parse::<u64>() else {
        return Headroom::Unknown;
    };
    // v1 represents an unlimited byte counter as a page-rounded signed maximum.
    // Values this large cannot bind physical memory on supported Linux machines.
    if version == Version::V1 && limit >= (1_u64 << 60) {
        return Headroom::Unlimited;
    }
    match read(&group.join(usage_name))
        .ok()
        .and_then(|usage| usage.trim().parse::<u64>().ok())
    {
        Some(usage) => Headroom::Finite(limit.saturating_sub(usage)),
        None => Headroom::Unknown,
    }
}

fn missing_controller(
    read: &mut impl FnMut(&Path) -> io::Result<String>,
    group: &Path,
    version: Version,
    global_root: bool,
    usage_name: &str,
) -> Headroom {
    // A missing limit alone must not turn an unreadable finite cap into host
    // headroom. The v2 root/disabled controller has neither accounting file.
    if !matches!(read(&group.join(usage_name)), Err(e) if e.kind() == io::ErrorKind::NotFound) {
        return Headroom::Unknown;
    }
    if global_root {
        return Headroom::Unlimited;
    }
    if version == Version::V2
        && read(&group.join("cgroup.controllers"))
            .is_ok_and(|text| !text.split_whitespace().any(|name| name == "memory"))
    {
        return Headroom::Unlimited;
    }
    Headroom::Unknown
}

#[cfg(test)]
mod tests;

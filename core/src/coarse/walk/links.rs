//! First pass of a walk: which path owns the bytes of every hardlinked inode.

use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use super::Walk;
use crate::coarse::model::MAX_DEPTH;

/// (device, inode) → the smallest path that links the inode.
pub type Owners = HashMap<(u64, u64), PathBuf>;

struct Level {
    iter: fs::ReadDir,
    path: PathBuf,
}

/// First pass: record the smallest path of every inode that has several links.
pub fn link_owners(top: &Path, w: &Walk<'_>, owners: &mut Owners) {
    let Ok(iter) = fs::read_dir(top) else {
        return;
    };
    let mut levels = vec![Level {
        iter,
        path: top.to_path_buf(),
    }];
    loop {
        let depth = levels.len();
        let Some(level) = levels.last_mut() else {
            return;
        };
        let Some(Ok(entry)) = level.iter.next() else {
            levels.pop();
            continue;
        };
        let name = entry.file_name();
        if depth == 1 && !w.admits(&name) {
            continue;
        }
        w.seen.fetch_add(1, Ordering::Relaxed);
        let Ok(md) = entry.metadata() else {
            continue;
        };
        if !md.is_dir() {
            if md.nlink() > 1 {
                record_owner(owners, (md.dev(), md.ino()), level.path.join(&name));
            }
            continue;
        }
        let path = level.path.join(&name);
        if depth < MAX_DEPTH
            && !w.boundary.is_mount(&md, &path)
            && let Ok(iter) = fs::read_dir(&path)
        {
            levels.push(Level { iter, path });
        }
    }
}

fn record_owner(owners: &mut Owners, key: (u64, u64), path: PathBuf) {
    match owners.entry(key) {
        Entry::Occupied(mut held) => {
            if path.as_os_str().as_bytes() < held.get().as_os_str().as_bytes() {
                held.insert(path);
            }
        }
        Entry::Vacant(slot) => {
            slot.insert(path);
        }
    }
}

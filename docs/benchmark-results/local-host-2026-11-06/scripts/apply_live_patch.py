#!/usr/bin/env python3
"""Make the research probe usable on a live whole-disk root. Every edit is asserted."""
from pathlib import Path

SRC = Path('/tmp/fsbench/probe-bin/src')


def edit(name, pairs):
    p = SRC / name
    s = p.read_text()
    for old, new, count in pairs:
        found = s.count(old)
        assert found == count, (name, old[:60], found, count)
        s = s.replace(old, new)
    p.write_text(s)


edit('model.rs', [
    ('pub const MAX_RECORDS: usize = 100_000;', 'pub const MAX_RECORDS: usize = 5_000_000;', 1),
    ('pub const MAX_BUCKETS: usize = 400_000;', 'pub const MAX_BUCKETS: usize = 4_000_000;', 1),
])

edit('walk.rs', [
    # inspect: skip instead of failing
    ('''fn inspect(path: &Path, root_dev: u64, mounts: &HashSet<PathBuf>) -> R<fs::Metadata> {
    if mounts.contains(path) {
        return Err(format!("nested mount: {}", path.display()).into());
    }
    let m = fs::symlink_metadata(path)?;
    if m.dev() != root_dev {
        return Err(format!("cross-device path: {}", path.display()).into());
    }
    Ok(m)
}''',
     '''/// Live-system variant: the snapshot directory, mounts, other devices and entries that
/// vanish or cannot be read are skipped (and counted) instead of failing the scan.
pub static EXCLUDE: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
pub static TOLERATED: AtomicU64 = AtomicU64::new(0);
pub static CHANGED: AtomicBool = AtomicBool::new(false);
fn tolerate() {
    TOLERATED.fetch_add(1, Ordering::Relaxed);
}
fn inspect(path: &Path, root_dev: u64, mounts: &HashSet<PathBuf>) -> R<Option<fs::Metadata>> {
    if mounts.contains(path) || EXCLUDE.get().is_some_and(|x| x == path) {
        return Ok(None);
    }
    let Ok(m) = fs::symlink_metadata(path) else {
        tolerate();
        return Ok(None);
    };
    if m.dev() != root_dev {
        return Ok(None);
    }
    Ok(Some(m))
}''', 1),
    ('use std::{fs, io};', 'use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};\nuse std::fs;', 1),
    # root must exist
    ('    let meta = inspect(root, dev, mounts)?;\n', '    let meta = inspect(root, dev, mounts)?.ok_or("scan root unavailable")?;\n', 1),
    ('    let m = inspect(root, dev, mounts)?;\n', '    let m = inspect(root, dev, mounts)?.ok_or("scan root unavailable")?;\n', 1),
    # children: skip unavailable entries (both passes)
    ('        let m = inspect(&p, dev, mounts)?;\n', '        let Some(m) = inspect(&p, dev, mounts)? else {\n            continue;\n        };\n', 2),
    # pass 1: tolerate readdir errors and unreadable subdirectories
    ('        let entry = entry?;\n', '        let Ok(entry) = entry else {\n            tolerate();\n            continue;\n        };\n', 1),
    ('            stack.push(fs::read_dir(&p)?);\n',
     '            match fs::read_dir(&p) {\n                Ok(rd) => stack.push(rd),\n                Err(_) => tolerate(),\n            }\n', 1),
    # pass 2: tolerate readdir errors, treat unreadable directories as leaf entries
    ('            Some(r) => r?,\n', '            Some(Ok(e)) => e,\n            Some(Err(_)) => {\n                tolerate();\n                continue;\n            }\n', 1),
    ('''            stack.push(Frame {
                node: n,
                iter: fs::read_dir(p)?,
                slot,
                slots: Slots::new(),
            });
        } else {
            frame.node.total.add(&n.total)?;
            if n.total.allocated >= threshold {
                retain(&mut retained, n)?;
            } else {
                frame.slots.entry(slot).or_default().add(&n.total)?;
            }
        }
''',
     '''            match fs::read_dir(&p) {
                Ok(iter) => {
                    stack.push(Frame {
                        node: n,
                        iter,
                        slot,
                        slots: Slots::new(),
                    });
                    continue;
                }
                Err(_) => tolerate(),
            }
        }
        frame.node.total.add(&n.total)?;
        if n.total.allocated >= threshold {
            retain(&mut retained, n)?;
        } else {
            frame.slots.entry(slot).or_default().add(&n.total)?;
        }
''', 1),
    ('        if digest.count > 10_000_000 {', '        if digest.count > 100_000_000 {', 2),
    # scan_inner: changes during the scan are recorded, not fatal
    ('''    if crate::mounts::nested(root)? != mounts {
        return Err("mount topology changed".into());
    }
''', '''    if crate::mounts::nested(root)? != mounts {
        CHANGED.store(true, Ordering::Relaxed);
    }
''', 2),
    ('''    if first != folded.digest {
        return Err(io::Error::other("filesystem changed between metadata passes").into());
    }
''', '''    if first != folded.digest {
        CHANGED.store(true, Ordering::Relaxed);
    }
''', 1),
])

edit('main.rs', [
    ('''    if snapshot.starts_with(&root) {
        return Err("snapshot directory must be outside scan root".into());
    }
''', '''    if snapshot.starts_with(&root) {
        // live-system variant: the walk skips the snapshot directory itself
        let _ = walk::EXCLUDE.set(snapshot.clone());
    }
''', 1),
    ('"elapsed_seconds":start.elapsed().as_secs_f64()})',
     '"elapsed_seconds":start.elapsed().as_secs_f64(),\n        "tolerated_errors":walk::TOLERATED.load(std::sync::atomic::Ordering::Relaxed),"digest_changed":walk::CHANGED.load(std::sync::atomic::Ordering::Relaxed)})', 1),
])
print('patch applied')

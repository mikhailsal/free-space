use crate::model::{MAX_DEPTH, MAX_RECORDS, Node, R, Stats, now};
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::{fs, io};

#[derive(Default, PartialEq, Eq)]
struct Digest {
    count: u64,
    xor: u64,
    sum: u64,
}
impl Digest {
    fn add(&mut self, p: &Path, m: &fs::Metadata) {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (
            p.as_os_str().as_bytes(),
            m.dev(),
            m.ino(),
            m.mode(),
            m.nlink(),
            m.len(),
            m.blocks(),
            m.mtime(),
            m.mtime_nsec(),
        )
            .hash(&mut h);
        let v = h.finish();
        self.count += 1;
        self.xor ^= v;
        self.sum = self.sum.wrapping_add(v);
    }
}
struct Frame {
    path: PathBuf,
    parent: Option<PathBuf>,
    node: Node,
    iter: fs::ReadDir,
}
pub struct ResultTree {
    pub nodes: Vec<Node>,
    pub visited: u64,
    pub has_hardlinks: bool,
}

fn inspect(path: &Path, root_dev: u64, mounts: &HashSet<PathBuf>) -> R<fs::Metadata> {
    if mounts.contains(path) {
        return Err(format!("nested mount: {}", path.display()).into());
    }
    let m = fs::symlink_metadata(path)?;
    if m.dev() != root_dev {
        return Err(format!("cross-device path: {}", path.display()).into());
    }
    Ok(m)
}
fn metadata_pass(
    root: &Path,
    dev: u64,
    mounts: &HashSet<PathBuf>,
) -> R<(HashMap<(u64, u64), PathBuf>, Digest)> {
    let mut owners = HashMap::new();
    let mut digest = Digest::default();
    let meta = inspect(root, dev, mounts)?;
    digest.add(root, &meta);
    let mut stack = vec![fs::read_dir(root)?];
    while let Some(top) = stack.last_mut() {
        let Some(entry) = top.next() else {
            stack.pop();
            continue;
        };
        let p = entry?.path();
        let m = inspect(&p, dev, mounts)?;
        digest.add(&p, &m);
        if digest.count > 10_000_000 {
            return Err("visited entry cap exceeded".into());
        }
        if !m.is_dir() && m.nlink() > 1 {
            owners
                .entry((m.dev(), m.ino()))
                .and_modify(|old: &mut PathBuf| {
                    if p.as_os_str().as_bytes() < old.as_os_str().as_bytes() {
                        *old = p.clone();
                    }
                })
                .or_insert(p.clone());
            if owners.len() > MAX_RECORDS {
                return Err("hardlink owner cap exceeded".into());
            }
        }
        if m.is_dir() {
            if stack.len() >= MAX_DEPTH {
                return Err("directory depth limit exceeded".into());
            }
            stack.push(fs::read_dir(&p)?);
        }
    }
    Ok((owners, digest))
}
fn node(
    path: PathBuf,
    parent: Option<PathBuf>,
    m: &fs::Metadata,
    owners: &HashMap<(u64, u64), PathBuf>,
    stamp: i64,
) -> Node {
    let kind = if m.is_dir() {
        1
    } else if m.is_file() {
        0
    } else {
        2
    };
    let alias = kind != 1 && m.nlink() > 1 && owners.get(&(m.dev(), m.ino())) != Some(&path);
    let (allocated, apparent) = if alias {
        (0, 0)
    } else {
        (m.blocks().saturating_mul(512), m.len())
    };
    let total = Stats {
        allocated,
        apparent,
        files: u64::from(kind == 0),
        dirs: u64::from(kind == 1),
        others: u64::from(kind == 2),
        errors: 0,
        min_scanned_at: Some(stamp),
        newest_mtime: if kind == 0 {
            Some(m.mtime().min(stamp))
        } else {
            None
        },
    };
    Node {
        path,
        parent,
        kind,
        own_allocated: allocated,
        own_apparent: apparent,
        own_errors: 0,
        total,
        rest: Stats::default(),
        scanned_at: stamp,
    }
}
fn finish(stack: &mut Vec<Frame>, retained: &mut Vec<Node>, threshold: u64) -> R<()> {
    let frame = stack.pop().ok_or("empty stack")?;
    let keep = frame.parent.is_none() || frame.node.total.allocated >= threshold;
    if let Some(parent) = stack.last_mut() {
        parent.node.total.add(&frame.node.total)?;
        if !keep {
            parent.node.rest.add(&frame.node.total)?;
        }
    }
    if keep {
        retained.push(frame.node);
        if retained.len() > MAX_RECORDS {
            return Err("retained record cap exceeded".into());
        }
    }
    Ok(())
}
fn fold_pass(
    root: &Path,
    dev: u64,
    threshold: u64,
    owners: &HashMap<(u64, u64), PathBuf>,
    mounts: &HashSet<PathBuf>,
) -> R<(Vec<Node>, Digest)> {
    let mut retained = Vec::new();
    let mut digest = Digest::default();
    let stamp = now();
    let m = inspect(root, dev, mounts)?;
    digest.add(root, &m);
    let mut stack = vec![Frame {
        path: root.to_path_buf(),
        parent: None,
        node: node(root.to_path_buf(), None, &m, owners, stamp),
        iter: fs::read_dir(root)?,
    }];
    while let Some(frame) = stack.last_mut() {
        let entry = match frame.iter.next() {
            Some(r) => r?,
            None => {
                finish(&mut stack, &mut retained, threshold)?;
                continue;
            }
        };
        let p = entry.path();
        let m = inspect(&p, dev, mounts)?;
        digest.add(&p, &m);
        if digest.count > 10_000_000 {
            return Err("visited entry cap exceeded".into());
        }
        let par = frame.path.clone();
        let n = node(p.clone(), Some(par.clone()), &m, owners, stamp);
        if m.is_dir() {
            if stack.len() >= MAX_DEPTH {
                return Err("directory depth limit exceeded".into());
            }
            stack.push(Frame {
                path: p.clone(),
                parent: Some(par),
                node: n,
                iter: fs::read_dir(p)?,
            });
        } else {
            frame.node.total.add(&n.total)?;
            if n.total.allocated >= threshold {
                retained.push(n);
                if retained.len() > MAX_RECORDS {
                    return Err("retained record cap exceeded".into());
                }
            } else {
                frame.node.rest.add(&n.total)?;
            }
        }
    }
    Ok((retained, digest))
}
pub fn scan(root: &Path, threshold: u64) -> R<ResultTree> {
    let meta = fs::symlink_metadata(root)?;
    if !meta.is_dir() {
        return Err("root must be a directory".into());
    }
    let mounts = crate::mounts::nested(root)?;
    let (owners, first) = metadata_pass(root, meta.dev(), &mounts)?;
    if crate::mounts::nested(root)? != mounts {
        return Err("mount topology changed".into());
    }
    let (nodes, second) = fold_pass(root, meta.dev(), threshold, &owners, &mounts)?;
    if crate::mounts::nested(root)? != mounts {
        return Err("mount topology changed".into());
    }
    if first != second {
        return Err(io::Error::other("filesystem changed between metadata passes").into());
    }
    Ok(ResultTree {
        nodes,
        visited: first.count + second.count,
        has_hardlinks: !owners.is_empty(),
    })
}

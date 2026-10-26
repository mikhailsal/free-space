use super::*;
use std::ffi::OsString;

pub(super) fn read_dir(ctx: &Ctx, job: &Job) -> (DirResult, Vec<Job>) {
    let mut res = DirResult {
        id: job.id,
        base: 0,
        scanned_at: platform::now_secs(),
        denied: false,
        errors: 0,
        names: Vec::new(),
        children: Vec::new(),
        hardlinks: Vec::new(),
        mounts: Vec::new(),
    };
    let rd = match fs::read_dir(&job.path) {
        Ok(rd) => rd,
        Err(_) => {
            res.denied = true;
            res.errors = 1;
            return (res, Vec::new());
        }
    };
    let mut ents: Vec<(OsString, io::Result<fs::Metadata>, Option<fs::FileType>)> = Vec::new();
    for e in rd {
        match e {
            Ok(e) => {
                let md = e.metadata();
                let ft = if md.is_err() {
                    e.file_type().ok()
                } else {
                    None
                };
                ents.push((e.file_name(), md, ft));
            }
            Err(_) => {
                // readdir failed mid-stream: the listing is incomplete.
                res.denied = true;
                res.errors += 1;
                break;
            }
        }
    }
    finish_directory(ctx, job, res, ents)
}

fn finish_directory(
    ctx: &Ctx,
    job: &Job,
    mut res: DirResult,
    mut ents: Vec<(OsString, io::Result<fs::Metadata>, Option<fs::FileType>)>,
) -> (DirResult, Vec<Job>) {
    ents.sort_unstable_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));

    let n = ents.len() as u32;
    let base = ctx.next_id.fetch_add(n, Ordering::Relaxed);
    if base.checked_add(n).is_none_or(|e| e == u32::MAX) {
        ctx.overflow.store(1, Ordering::Relaxed);
        res.denied = true;
        return (res, Vec::new());
    }
    res.base = base;
    ctx.seen.fetch_add(n as u64, Ordering::Relaxed);
    let mut sub = Vec::new();
    res.children.reserve(ents.len());
    for (i, (name, md, ft)) in ents.into_iter().enumerate() {
        process_entry(
            ctx,
            job,
            base + i as u32,
            name,
            md,
            ft,
            (&mut res, &mut sub),
        );
    }
    // LIFO stack pops the last job first: push in reverse so siblings run in name order.
    sub.reverse();
    (res, sub)
}

fn process_entry(
    ctx: &Ctx,
    job: &Job,
    id: u32,
    name: OsString,
    md: io::Result<fs::Metadata>,
    ft: Option<fs::FileType>,
    output: (&mut DirResult, &mut Vec<Job>),
) {
    let (res, sub) = output;
    let nb = name.as_bytes();
    let name_start = res.names.len() as u32;
    let name_len = nb.len().min(u16::MAX as usize) as u16;
    res.names.extend_from_slice(&nb[..name_len as usize]);
    let mut rec = ChildRec {
        name_start,
        name_len,
        kind: Kind::Other.code(),
        flags: 0,
        allocated: 0,
        apparent: 0,
        mtime: 0,
        nlink: 1,
        errors: 0,
    };
    match md {
        Ok(md) => {
            let t = md.file_type();
            let mut kind = if t.is_dir() {
                Kind::Dir
            } else if t.is_file() {
                Kind::File
            } else if t.is_symlink() {
                Kind::Symlink
            } else {
                Kind::Other
            };
            rec.allocated = md.blocks() * 512;
            rec.apparent = md.len();
            rec.mtime = md.mtime();
            rec.nlink = md.nlink().min(u32::MAX as u64) as u32;
            if kind == Kind::Dir {
                if record_directory(ctx, job, id, &name, &md, &mut rec, (res, sub)) {
                    kind = Kind::Mount;
                }
            } else if md.nlink() > 1 {
                res.hardlinks.push((id, md.dev(), md.ino()));
            }
            rec.kind = kind.code();
        }
        Err(_) => {
            let kind = match ft {
                Some(t) if t.is_dir() => Kind::Dir,
                Some(t) if t.is_file() => Kind::File,
                Some(t) if t.is_symlink() => Kind::Symlink,
                Some(t) if t.is_fifo() || t.is_socket() => Kind::Other,
                _ => Kind::Other,
            };
            rec.kind = kind.code();
            rec.flags |= flags::STAT_ERROR;
            if kind == Kind::Dir {
                rec.flags |= flags::DENIED;
            }
            rec.errors = 1;
        }
    }
    res.children.push(rec);
}

fn record_directory(
    ctx: &Ctx,
    job: &Job,
    id: u32,
    name: &OsString,
    metadata: &fs::Metadata,
    rec: &mut ChildRec,
    output: (&mut DirResult, &mut Vec<Job>),
) -> bool {
    let (res, sub) = output;
    let root = &ctx.roots[job.root as usize];
    let child_path = job.path.join(name);
    let crosses = metadata.dev() != root.dev;
    if (crosses && ctx.one_file_system) || root.bind_mounts.contains(&child_path) {
        rec.allocated = 0;
        rec.apparent = 0;
        res.mounts.push((id, child_path));
        true
    } else {
        sub.push(Job {
            id,
            root: job.root,
            path: child_path,
        });
        false
    }
}

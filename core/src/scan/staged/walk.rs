use super::*;
use std::ffi::OsString;
use std::fs::{self, Metadata};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::MetadataExt;

pub(super) struct Job {
    id: u64,
    root: usize,
    path: PathBuf,
}

impl StagedTree {
    pub(super) fn job(&self, id: u64, root: u64, path: &Path) -> Result<()> {
        self.conn
            .prepare_cached("INSERT INTO jobs VALUES (?1,?2,?3)")?
            .execute(params![id as i64, root as i64, path.as_os_str().as_bytes()])?;
        Ok(())
    }

    pub(super) fn next_job(&self) -> Result<Option<Job>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id,root,path FROM jobs ORDER BY id DESC LIMIT 1",
                [],
                |r| {
                    Ok(Job {
                        id: r.get::<_, i64>(0)? as u64,
                        root: r.get::<_, i64>(1)? as usize,
                        path: PathBuf::from(OsString::from_vec(r.get(2)?)),
                    })
                },
            )
            .optional()?)
    }

    pub(super) fn read_directory(&mut self, job: Job, ctx: &mut WalkContext<'_>) -> Result<()> {
        let scanned_at = platform::now_secs();
        let mut errors = 0;
        let mut denied = false;
        let entries = match fs::read_dir(&job.path) {
            Ok(entries) => entries,
            Err(_) => return self.finish_directory(job.id, scanned_at, 1, true, ctx.guard),
        };
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => {
                    errors += 1;
                    denied = true;
                    break;
                }
            };
            self.read_entry(&job, entry, scanned_at, ctx)?;
        }
        self.finish_directory(job.id, scanned_at, errors, denied, ctx.guard)
    }

    fn finish_directory(
        &self,
        id: u64,
        scanned_at: i64,
        errors: u32,
        denied: bool,
        guard: &mut SpaceGuard,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE raw SET scanned_at=max(scanned_at,?2), errors=errors+?3,
            flags=flags | ?4 WHERE id=?1",
            params![
                id as i64,
                scanned_at,
                errors,
                if denied { flags::DENIED } else { 0 }
            ],
        )?;
        self.conn
            .execute("DELETE FROM jobs WHERE id=?1", [id as i64])?;
        guard.tick()?;
        Ok(())
    }

    fn read_entry(
        &mut self,
        job: &Job,
        entry: fs::DirEntry,
        scanned_at: i64,
        ctx: &mut WalkContext<'_>,
    ) -> Result<()> {
        let path = job.path.join(entry.file_name());
        if path == ctx.stage_path {
            return Ok(());
        }
        if self.count >= u32::MAX as u64 - 1 {
            return Err(Error::generic("more than 4 billion entries"));
        }
        let metadata = entry.metadata();
        let file_type = if metadata.is_err() {
            entry.file_type().ok()
        } else {
            None
        };
        let mut node = super::super::walk::entry_metadata(&metadata, file_type);
        node.scanned_at = scanned_at;
        if let Ok(md) = &metadata {
            self.record_entry(job, &path, md, &mut node, ctx)?;
        }
        self.insert(
            self.count,
            Some(job.id),
            entry.file_name().as_bytes(),
            &node,
        )?;
        self.count += 1;
        ctx.opts.seen.fetch_add(1, Ordering::Relaxed);
        ctx.guard.tick()?;
        Ok(())
    }

    fn record_entry(
        &self,
        job: &Job,
        path: &Path,
        md: &Metadata,
        node: &mut Node,
        ctx: &WalkContext<'_>,
    ) -> Result<()> {
        let id = self.count;
        let root = &ctx.roots[job.root];
        if node.kind() == Kind::Dir {
            let crosses = md.dev() != root.dev;
            if (crosses && self.one_file_system) || root.bind_mounts.contains(path) {
                node.kind = Kind::Mount.code();
                node.allocated = 0;
                node.apparent = 0;
                let fs_type = ctx
                    .mounts
                    .iter()
                    .rev()
                    .find(|m| m.mount_point == path)
                    .map(|m| m.fs_type.as_str())
                    .unwrap_or("");
                let stats = super::super::walk::statvfs_safe(fs_type)
                    .then(|| platform::statvfs(path).ok())
                    .flatten();
                self.conn
                    .prepare_cached("INSERT INTO mount_info VALUES (?1,?2,?3,?4,?5)")?
                    .execute(params![
                        id as i64,
                        path.as_os_str().as_bytes(),
                        fs_type,
                        stats.map(|s| s.used as i64),
                        stats.map(|s| s.total as i64)
                    ])?;
            } else {
                self.job(id, job.root as u64, path)?;
            }
        } else if md.nlink() > 1 {
            self.conn
                .prepare_cached("INSERT INTO links VALUES (?1,?2,?3,?4)")?
                .execute(params![
                    id as i64,
                    md.dev() as i64,
                    md.ino() as i64,
                    path.as_os_str().as_bytes()
                ])?;
        }
        Ok(())
    }
}

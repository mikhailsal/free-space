//! Full-scan and local-refresh publication; v1 production writers are separate.
use super::*;

impl Store {
    /// Replaces an existing directory, including a configured root. Missing
    /// non-root directories are removed; root deletion still requires a full scan.
    pub fn refresh(&mut self, path: &Path, workers: usize) -> Result<WriteReport> {
        let prepared = self.prepare_refresh(path, workers)?;
        self.publish_refresh(prepared)
    }
    pub(super) fn prepare_refresh(
        &mut self,
        path: &Path,
        workers: usize,
    ) -> Result<PreparedRefresh> {
        let bytes = path.as_os_str().as_bytes();
        validate_path(bytes)?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Deferred)?;
        let pair = pair(&tx)?.ok_or_else(|| unsupported("seed the store first"))?;
        let target = live_path(&tx, bytes)?
            .ok_or_else(|| Error::usage("path is absent from current generation"))?;
        if target.kind != Kind::Dir {
            return Err(Error::usage(
                "refresh requires a directory within its root filesystem",
            ));
        }
        let mut root = metadata::live_root(&tx, &target.scope)?;
        let one_file_system: bool = tx.query_row(
            "SELECT one_file_system FROM control WHERE singleton=1",
            [],
            |r| r.get(0),
        )?;
        tx.commit()?;
        let target_meta = match std::fs::symlink_metadata(path) {
            Ok(m) if m.is_dir() => Some(m),
            Ok(_) => {
                return Err(Error::usage(
                    "refresh directory replaced by non-directory; refresh its parent",
                ));
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(Error::io("refresh path", e)),
        };
        crate::refresh::mounts::check(
            path,
            target_meta.as_ref().map(|m| m.dev()),
            &[root.info()],
            one_file_system,
            &String::from_utf8_lossy(bytes),
        )?;
        if target_meta.is_none() && target.parent.is_none() {
            return Err(Error::usage("root is missing; run a full scan"));
        }
        let started = crate::platform::now_secs();
        let start = Instant::now();
        let staged = if target_meta.is_some() {
            let mut opts = WalkOptions::new(vec![path.to_path_buf()]);
            opts.workers = workers;
            opts.one_file_system = one_file_system;
            stage(
                scan::walk(&opts).map_err(|e| Error::io("v2 refresh walk", e))?,
                Some(&target.scope),
            )?
        } else {
            staging::Staged {
                entries: Vec::new(),
                roots: Vec::new(),
                mounts: Vec::new(),
            }
        };
        if staged.entries.first().is_some_and(|e| e.path != bytes) {
            return Err(Error::usage("refresh path resolves to another location"));
        }
        // Preserve the original root type and refresh its df numbers as v1 does.
        if scan::walk::statvfs_safe(&root.fs_type)
            && let Ok(stats) = crate::platform::statvfs(&root.info().path)
        {
            root.stats = stats;
        }
        if target.parent.is_none()
            && let Some(m) = target_meta
        {
            root.dev = m.dev();
        }
        Ok(PreparedRefresh {
            base_generation: pair.current,
            target,
            root,
            staged,
            started,
            walk: start.elapsed(),
        })
    }
    pub(super) fn publish_refresh(&mut self, prepared: PreparedRefresh) -> Result<WriteReport> {
        let PreparedRefresh {
            base_generation,
            target,
            root,
            mut staged,
            started,
            walk,
        } = prepared;
        let start = Instant::now();
        let before = self.conn.total_changes();
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        retention::admit_wal(&tx, &self.path)?;
        if pair(&tx)?.map(|p| p.current) != Some(base_generation) {
            return Err(Error::new(
                ErrorCode::Rotated,
                "v2 generation changed during refresh walk; retry against the new generation",
            ));
        }
        let generation = next_generation(Some(base_generation))?;
        let old = old_subtree(&tx, &target)?;
        let links = hardlinks::refresh(&tx, &old, &mut staged.entries)?;
        fold(&mut staged.entries)?;
        let mut changes = vec![(
            target.parent,
            aggregate::Delta::between(&target, staged.entries.first()),
        )];
        for (previous, replacement) in links.entries {
            changes.push((
                previous.parent,
                aggregate::Delta::between(&previous, Some(&replacement)),
            ));
            retire(&tx, previous.id, generation)?;
            insert(&tx, &replacement, generation)?;
        }
        for e in &old {
            retire(&tx, e.id, generation)?;
            if e.kind == Kind::Mount {
                metadata::retire_mount(&tx, e.id, generation)?;
            }
        }
        let ids = assign_ids(&tx, &mut staged.entries, target.parent)?;
        for e in &staged.entries {
            insert(&tx, e, generation)?;
        }
        for mut mount in staged.mounts {
            mount.entry_id = ids[&mount.entry_id];
            metadata::insert_mount(&tx, &mount, generation)?;
        }
        let ancestors = aggregate::update(&tx, changes, generation)?;
        metadata::refresh(
            &tx,
            &root,
            base_generation,
            generation,
            &target.path,
            started,
            staged.entries.len() as i64 - old.len() as i64,
        )?;
        retention::admit_rows(&tx, false, generation)?;
        tx.commit()?;
        Ok(WriteReport {
            generation,
            old_subtree_entries: old.len(),
            new_subtree_entries: staged.entries.len(),
            ancestors,
            touched_inode_groups: links.groups,
            known_inode_members: links.members,
            sql_changes: self.conn.total_changes() - before,
            walk,
            transaction: start.elapsed(),
        })
    }
}

fn old_subtree(conn: &Connection, target: &Entry) -> Result<Vec<Entry>> {
    let upper = if target.sort_key.is_empty() {
        vec![255]
    } else {
        prefix_end(&target.sort_key)
    };
    let mut q = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM live WHERE scope=?1 AND sort_key>=?2 AND sort_key<?3"
    ))?;
    Ok(
        q.query_map(params![target.scope, target.sort_key, upper], Entry::row)?
            .collect::<rusqlite::Result<Vec<_>>>()?,
    )
}

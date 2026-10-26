//! Full scan profiles share one atomic publication contract.
use super::*;
use crate::scan::staged::StagedTree;
use crate::snapshot::writer::Reserve;

struct Seed {
    scanned_at: i64,
    count: usize,
    one_file_system: bool,
    keep_previous: bool,
}
impl Store {
    pub fn scan(&mut self, root: &Path, workers: usize) -> Result<WriteReport> {
        self.scan_roots(&[root.to_path_buf()], true, workers)
    }
    /// Root normalization and overlap validation are shared with the v1 walker.
    pub fn scan_roots(
        &mut self,
        roots: &[PathBuf],
        one_file_system: bool,
        workers: usize,
    ) -> Result<WriteReport> {
        self.scan_roots_with_options(roots, one_file_system, workers, false, true)
    }
    /// Disk staging bounds scan memory by SQLite cache plus traversal depth.
    /// Disabling `keep_previous` removes the public comparison generation;
    /// existing readers remain pinned and obsolete versions await maintenance.
    pub fn scan_roots_with_options(
        &mut self,
        roots: &[PathBuf],
        one_file_system: bool,
        workers: usize,
        low_memory: bool,
        keep_previous: bool,
    ) -> Result<WriteReport> {
        if roots.is_empty() {
            return Err(Error::usage("at least one scan root is required"));
        }
        let start = Instant::now();
        let mut opts = WalkOptions::new(roots.to_vec());
        opts.workers = workers;
        opts.one_file_system = one_file_system;
        if low_memory {
            let dir = self.path.parent().expect("canonical database parent");
            let path = dir.join(format!(
                "scan-{}.stage.tmp",
                crate::snapshot::writer::snapshot_id()
            ));
            let staged = StagedTree::walk(&opts, &path, &Reserve::new(dir))?;
            self.seed_disk(&staged, one_file_system, keep_previous, start.elapsed())
        } else {
            let tree = scan::walk(&opts).map_err(scan::walk_error)?;
            self.seed_memory(tree, one_file_system, keep_previous, start)
        }
    }
    #[cfg(test)]
    pub(crate) fn seed_tree(
        &mut self,
        tree: Tree,
        one_file_system: bool,
        start: Instant,
    ) -> Result<WriteReport> {
        self.seed_memory(tree, one_file_system, true, start)
    }
    fn seed_memory(
        &mut self,
        tree: Tree,
        one_file_system: bool,
        keep_previous: bool,
        start: Instant,
    ) -> Result<WriteReport> {
        let scanned_at = tree.started_at;
        let mut staged = stage(tree, None)?;
        hardlinks::scan(&mut staged.entries);
        fold(&mut staged.entries)?;
        let seed = Seed {
            scanned_at,
            count: staged.entries.len(),
            one_file_system,
            keep_previous,
        };
        self.publish_seed(seed, start.elapsed(), |tx, generation| {
            let ids = assign_ids(tx, &mut staged.entries, None)?;
            for entry in &staged.entries {
                insert(tx, entry, generation)?;
            }
            for mut root in staged.roots {
                root.entry_id = ids[&root.entry_id];
                metadata::insert_root(tx, &root, generation)?;
            }
            for mut mount in staged.mounts {
                mount.entry_id = ids[&mount.entry_id];
                metadata::insert_mount(tx, &mount, generation)?;
            }
            Ok(())
        })
    }
    pub(crate) fn seed_disk(
        &mut self,
        staged: &StagedTree,
        one_file_system: bool,
        keep_previous: bool,
        walk: Duration,
    ) -> Result<WriteReport> {
        self.conn.execute_batch("PRAGMA cache_size=-8192; PRAGMA temp_store=FILE; PRAGMA mmap_size=0; PRAGMA cache_spill=ON;")?;
        let seed = Seed {
            scanned_at: staged.started_at(),
            count: staged.len() as usize,
            one_file_system,
            keep_previous,
        };
        self.publish_seed(seed, walk, |tx, generation| {
            staged.visit(&mut disk::Sink {
                conn: tx,
                generation,
            })
        })
    }
    fn publish_seed(
        &mut self,
        seed: Seed,
        walk: Duration,
        write: impl FnOnce(&Connection, i64) -> Result<()>,
    ) -> Result<WriteReport> {
        let start = Instant::now();
        let before = self.conn.total_changes();
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        retention::admit_wal(&tx, &self.path)?;
        let current = pair(&tx)?.map(|p| p.current);
        let generation = next_generation(current)?;
        let old_count: i64 = tx.query_row("SELECT count(*) FROM live", [], |r| r.get(0))?;
        for table in ["versions", "root_versions", "mount_versions"] {
            tx.execute(
                &format!("UPDATE {table} SET dead=?1 WHERE dead IS NULL"),
                [generation],
            )?;
        }
        tx.execute_batch("DELETE FROM live; DELETE FROM live_roots; DELETE FROM live_mounts;")?;
        write(&tx, generation)?;
        let previous = current.filter(|_| seed.keep_previous);
        publish(&tx, &seed, generation, previous)?;
        retention::admit_rows(&tx, true, generation)?;
        tx.commit()?;
        Ok(WriteReport {
            generation,
            old_subtree_entries: old_count as usize,
            new_subtree_entries: seed.count,
            ancestors: 0,
            touched_inode_groups: 0,
            known_inode_members: 0,
            sql_changes: self.conn.total_changes() - before,
            walk,
            transaction: start.elapsed(),
        })
    }
}
fn publish(conn: &Connection, seed: &Seed, generation: i64, previous: Option<i64>) -> Result<()> {
    metadata::insert_generation(
        conn,
        &crate::snapshot::reader::Meta {
            schema_version: 2,
            generation: generation as u64,
            prev_generation: previous.map(|v| v as u64),
            scanned_at: seed.scanned_at,
            host: crate::platform::hostname(),
            os: crate::platform::os_name(),
            snapshot_id: crate::snapshot::writer::snapshot_id(),
            entries: seed.count as u64,
            one_file_system: seed.one_file_system,
            freespace_version: crate::VERSION.into(),
        },
        generation,
    )?;
    conn.execute("INSERT INTO control VALUES(1,?1,?2,?1,?3) ON CONFLICT(singleton) DO UPDATE SET current=excluded.current,prev=excluded.prev,full_scan=excluded.full_scan,one_file_system=excluded.one_file_system",params![generation,previous,seed.one_file_system])?;
    Ok(())
}

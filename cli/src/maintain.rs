//! Explicit bounded v2 reclamation; policy updates and cleanup are separate.
use crate::{backend, output};
use clap::Args;
use freespace_core::{
    Error, ErrorCode,
    config::Backend,
    snapshot::{lock, v2::Store},
};
use serde::Serialize;

#[derive(Args)]
pub struct MaintainArgs {
    /// Maximum records examined by this cleanup batch (1..100000)
    #[arg(long, default_value_t = 1000, value_parser = clap::value_parser!(u32).range(1..=100000))]
    pub budget: u32,
    /// Request a WAL checkpoint; this may perform global work beyond --budget
    #[arg(long)]
    pub checkpoint: bool,
    /// Persist the retained-record admission limit, even if cleanup is deferred
    #[arg(long, value_parser = clap::value_parser!(u64).range(1..=i64::MAX as u64))]
    pub max_records: Option<u64>,
    /// Persist the WAL admission threshold in bytes, even if cleanup is deferred
    #[arg(long, value_parser = clap::value_parser!(u64).range(1..=i64::MAX as u64))]
    pub max_wal_bytes: Option<u64>,
    #[arg(long)]
    pub json: bool,
}

#[derive(Serialize)]
struct Report {
    generation: Option<i64>,
    prev_generation: Option<i64>,
    budget: u32,
    checkpoint_requested: bool,
    policy_updated: bool,
    max_records: u64,
    max_wal_bytes: u64,
    readers_blocking: bool,
    checkpoint_busy: bool,
    examined: usize,
    deleted: usize,
    retained_rows: u64,
    wal_bytes: u64,
    checkpoint_log_frames: i64,
    checkpointed_frames: i64,
}

pub fn run(args: &MaintainArgs) -> output::Result<()> {
    let loaded = backend::load()?;
    if loaded.config.backend != Backend::V2 {
        return Err(Error::new(
            ErrorCode::NotSupported,
            "maintain requires the v2 backend; pass --backend v2 or configure backend = \"v2\"",
        )
        .into());
    }
    let dir = backend::dir(&loaded);
    // Missing/invalid stores fail before even creating the application lock.
    generation_pair(&dir)?;
    let _lock = lock::acquire_with_path_arg(
        &dir,
        "maintain",
        &backend::database(&dir).display().to_string(),
        None,
    )?;
    // Recheck under the writer lock, then drop the lease before reclamation.
    let generations = generation_pair(&dir)?;
    let mut store = Store::open(&backend::database(&dir))?;
    let mut policy = store.retention_policy()?;
    let updated = args.max_records.is_some() || args.max_wal_bytes.is_some();
    if updated {
        policy.max_records = args.max_records.unwrap_or(policy.max_records);
        policy.max_wal_bytes = args.max_wal_bytes.unwrap_or(policy.max_wal_bytes);
        store.configure_retention(policy)?;
        eprintln!(
            "v2 retention policy saved: max_records={}, max_wal_bytes={} (persists if maintenance is deferred or fails)",
            policy.max_records, policy.max_wal_bytes
        );
    }
    let report = store.maintain(args.budget as usize, args.checkpoint)?;
    let report = Report {
        generation: generations.map(|pair| pair.current),
        prev_generation: generations.and_then(|pair| pair.prev),
        budget: args.budget,
        checkpoint_requested: args.checkpoint,
        policy_updated: updated,
        max_records: policy.max_records,
        max_wal_bytes: policy.max_wal_bytes,
        readers_blocking: report.readers_blocking,
        checkpoint_busy: report.checkpoint_busy,
        examined: report.examined,
        deleted: report.deleted,
        retained_rows: report.retained_rows,
        wal_bytes: report.wal_bytes,
        checkpoint_log_frames: report.checkpoint_log_frames,
        checkpointed_frames: report.checkpointed_frames,
    };
    if args.json {
        output::print_json(&report)?;
    } else {
        print_report(&report)?;
    }
    Ok(())
}

/// Reader validates the schema before returning NoSnapshot for an unpublished
/// store. Such a store needs policy recovery too; an absent file must stay absent.
fn generation_pair(
    dir: &freespace_core::snapshot::SnapshotDir,
) -> freespace_core::Result<Option<freespace_core::snapshot::v2::GenerationPair>> {
    match backend::reader(dir) {
        Ok(reader) => Ok(Some(reader.generations)),
        Err(error)
            if error.code == ErrorCode::NoSnapshot
                && std::fs::metadata(backend::database(dir)).is_ok_and(|meta| meta.is_file()) =>
        {
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

fn print_report(report: &Report) -> std::io::Result<()> {
    let deferred = if report.readers_blocking {
        " · deferred: readers active"
    } else if report.checkpoint_busy {
        " · checkpoint busy"
    } else {
        ""
    };
    output::print(&format!(
        "v2 maintenance: {} examined, {} deleted, {} retained records, {} WAL bytes{deferred}\npolicy: max_records={}, max_wal_bytes={}{}\n",
        report.examined,
        report.deleted,
        report.retained_rows,
        report.wal_bytes,
        report.max_records,
        report.max_wal_bytes,
        if report.policy_updated {
            " (saved; applies even when cleanup is deferred)"
        } else {
            ""
        },
    ))
}

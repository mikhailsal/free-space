use super::*;

pub(super) fn run(dir: &SnapshotDir, entries: u64, factor: u64) -> Result<publish::PrecheckInfo> {
    let current = std::fs::metadata(dir.current())
        .map(|m| m.len())
        .unwrap_or(0);
    let estimate = entries
        .checked_mul(publish::BYTES_PER_ENTRY_ESTIMATE)
        .map(|n| n.max(current));
    let safe = estimate
        .and_then(|n| n.checked_mul(factor))
        .and_then(|n| n.checked_add(crate::snapshot::RESERVE_BYTES));
    let result = if safe.is_some() {
        publish::precheck(dir, Some(entries), factor)
    } else {
        Err(Error::new(
            ErrorCode::Space,
            "refresh space estimate overflow",
        ))
    };
    result.map_err(|mut e| {
        e.last_error = Some(state::LastError {
            ts: platform::now_secs(),
            op: "precheck".into(),
            message: e.message.clone(),
        });
        e
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overflowing_stage_estimate_is_a_precheck_error() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = SnapshotDir::new(tmp.path());
        let error = run(&dir, u64::MAX, 6).unwrap_err();
        assert_eq!(error.code, ErrorCode::Space);
        assert_eq!(error.last_error.as_ref().unwrap().op, "precheck");
    }
}

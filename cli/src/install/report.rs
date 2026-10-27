//! Installation result serialization and terminal output.

use freespace_core::systemd::TIMER;
use serde::Serialize;

use crate::output;

#[derive(Serialize)]
pub(super) struct Report {
    pub(super) binary: String,
    /// Where the binary was copied from; null when it already ran from there.
    pub(super) copied_from: Option<String>,
    pub(super) config: String,
    pub(super) roots: Vec<String>,
    pub(super) interval: String,
    pub(super) at: String,
    pub(super) keep_previous: bool,
    pub(super) low_memory: bool,
    pub(super) low_memory_reason: String,
    pub(super) snapshot_dir: String,
    pub(super) user: Option<String>,
    /// "acl", "group" (re-login needed) or "none".
    pub(super) access: String,
    pub(super) relogin_needed: bool,
    pub(super) next_run: Option<String>,
    pub(super) scan_started: bool,
    pub(super) scan_reason: Option<String>,
}

pub(super) fn print(report: &Report, json: bool) -> output::Result<()> {
    if json {
        output::print_json(report)?;
        return Ok(());
    }
    let mut out = String::new();
    out.push_str(&format!(
        "freespace {} timer installed\n  config     {} (roots {}, {} at {}, keep_previous {})\n",
        freespace_core::VERSION,
        output::escape(&report.config),
        output::escape(&report.roots.join(" ")),
        output::escape(&report.interval),
        output::escape(&report.at),
        report.keep_previous
    ));
    out.push_str(&format!(
        "  low_memory {} ({})\n",
        report.low_memory,
        output::escape(&report.low_memory_reason)
    ));
    let who = match (&report.user, report.access.as_str()) {
        (Some(u), "acl") => format!("read access for {} (ACL)", output::escape(u)),
        (Some(u), "group") => format!("read access for {} via group freespace", output::escape(u)),
        _ => "root only".into(),
    };
    out.push_str(&format!(
        "  snapshots  {} ({who})\n",
        output::escape(&report.snapshot_dir)
    ));
    out.push_str(&format!(
        "  timer      {TIMER}, next run {}\n",
        output::escape(report.next_run.as_deref().unwrap_or("unknown"))
    ));
    if report.relogin_needed {
        out.push_str(&format!(
            "Log out and back in once so {} picks up the freespace group.\n",
            output::escape(report.user.as_deref().unwrap_or("the user"))
        ));
    }
    match (report.scan_started, report.scan_reason.as_deref()) {
        (true, Some(why)) => out.push_str(&format!(
            "First scan started ({}) · Open: freespace gui\n",
            output::escape(why)
        )),
        (false, Some(why)) => out.push_str(&format!("{}\n", output::escape(why))),
        _ => {}
    }
    output::print(&out)?;
    Ok(())
}

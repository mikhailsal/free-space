//! Bare `freespace` (D10): the status sentence and three next commands, exit 0.
//! Piped, the next commands carry `--json` (D15); `--json` prints `status --json`.

use freespace_core::config;
use freespace_core::{ErrorCode, INSTALL_CMD, Result};

use crate::output;
use crate::status;

pub fn run(json: bool) -> Result<()> {
    let dir = config::load()?.snapshot_dir();
    let piped = !output::stdout_is_tty();
    let j = if piped { " --json" } else { "" };
    let s = match status::gather(&dir) {
        Ok(s) => s,
        Err(e) if e.code == ErrorCode::NoSnapshot && !json => {
            output::print(&format!(
                "No snapshot yet in {}.\nNext:\n{}",
                dir.path.display(),
                next(&[
                    (
                        INSTALL_CMD.to_owned(),
                        "install the nightly scan, start the first one"
                    ),
                    ("sudo freespace scan".into(), "or scan once by hand"),
                    ("freespace --help".into(), "every command"),
                ])
            ));
            return Ok(());
        }
        Err(e) => return Err(e),
    };
    if json {
        output::print_json(&s.status);
        return Ok(());
    }
    let r = &s.status;
    let at = shell_word(r.roots.first().map_or("/", |x| x.path.as_str()));
    let lines = if r.generation.is_none() {
        vec![
            (
                format!("freespace status{j}"),
                "progress; the first snapshot appears when the scan ends",
            ),
            ("freespace --help".into(), "every command"),
            (
                "freespace schema --json".into(),
                "what the --json answers look like",
            ),
        ]
    } else {
        let third = if r.prev.is_some() {
            (
                format!("freespace diff {at}{j}"),
                "what grew since the previous scan",
            )
        } else {
            (
                format!("freespace top {at} --kind dir --older-than 180d{j}"),
                "folders untouched for half a year",
            )
        };
        vec![
            (
                format!("freespace ls {at}{j}"),
                "biggest folders at the top",
            ),
            (
                format!("freespace top {at} --min-size 1G{j}"),
                "largest files",
            ),
            third,
        ]
    };
    let hint = status::doctor_hint(r)
        .map(|h| format!("{h}\n"))
        .unwrap_or_default();
    output::print(&format!(
        "{}\n{hint}Next:\n{}",
        status::sentence(&s),
        next(&lines)
    ));
    Ok(())
}

/// Quote a path for copy-paste into a shell when it needs it.
fn shell_word(p: &str) -> String {
    if p.bytes()
        .all(|b| b.is_ascii_alphanumeric() || b"/._-+,:@".contains(&b))
    {
        p.to_owned()
    } else {
        format!("'{}'", p.replace('\'', r"'\''"))
    }
}

fn next(lines: &[(String, &str)]) -> String {
    let w = lines
        .iter()
        .map(|(c, _)| c.chars().count())
        .max()
        .unwrap_or(0);
    lines
        .iter()
        .map(|(c, what)| format!("  {c:<w$}  {what}\n"))
        .collect()
}

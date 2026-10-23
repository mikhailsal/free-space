mod bare;
mod output;
mod query;
mod refresh;
mod scan;
mod schema;
mod status;

use std::process::ExitCode;

use clap::{CommandFactory, FromArgMatches, Parser, Subcommand};

const HELP_TEMPLATE: &str = "\
{before-help}{name} {version} · {about}

{usage-heading} {usage}

Look:
  ls              Children of a folder, biggest first
  top             Largest files, or folders untouched for a while
  find            Everything matching a filter, with the total size
  diff            What grew or shrank since the previous scan

Maintain:
  status          Snapshot age, free space, last error, running scan
  scan            Scan and publish a new snapshot (root)
  refresh         Rescan one folder into the current snapshot (root)

Setup:
  schema          JSON Schema of every --json answer

Run without a command for a one-line summary and next steps.
Every Look command takes --json. Errors under --json are JSON on stdout too.

{options}{after-help}";

#[derive(Parser)]
#[command(
    name = "freespace",
    version,
    about = "disk usage snapshot that is always ready",
    disable_help_subcommand = true,
    help_template = HELP_TEMPLATE
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
    /// Print the summary as JSON (same as `status --json`)
    #[arg(long)]
    json: bool,
}

#[derive(Subcommand)]
enum Command {
    /// Snapshot age, free space, last error, running scan
    #[command(after_help = "Examples:\n  freespace status\n  freespace status --json")]
    Status(status::StatusArgs),
    /// Children of a folder, biggest first
    #[command(
        after_help = "Examples:\n  freespace ls /home --limit 20\n  freespace ls ~/Downloads --sort mtime --json"
    )]
    Ls(query::LsArgs),
    /// Largest files, or folders untouched for a while
    #[command(
        after_help = "Examples:\n  freespace top / --min-size 1G\n  freespace top /home --kind dir --older-than 180d --json"
    )]
    Top(query::TopArgs),
    /// Everything matching a filter, with the total size
    #[command(
        after_help = "Examples:\n  freespace find /home --name '*.iso'\n  freespace find / --kind dir --name node_modules --json"
    )]
    Find(query::FindArgs),
    /// What grew or shrank since the previous scan
    #[command(
        after_help = "Examples:\n  freespace diff /\n  freespace diff /home/tass --kind any --json"
    )]
    Diff(query::DiffArgs),
    /// Scan and publish a new snapshot (root)
    #[command(after_help = "Examples:\n  sudo freespace scan\n  sudo freespace scan / --workers 4")]
    Scan(scan::ScanArgs),
    /// Rescan one folder into the current snapshot (root)
    #[command(
        after_help = "Examples:\n  sudo freespace refresh /home/tass/.cache\n  sudo freespace refresh /var/log --json"
    )]
    Refresh(refresh::RefreshArgs),
    /// JSON Schema of every --json answer
    #[command(after_help = "Examples:\n  freespace schema --json > schema.json")]
    Schema(schema::SchemaArgs),
}

fn main() -> ExitCode {
    let matches = match Cli::command().try_get_matches() {
        Ok(m) => m,
        Err(e) => return output::clap_error(e),
    };
    let cli = match Cli::from_arg_matches(&matches) {
        Ok(c) => c,
        Err(e) => return output::clap_error(e),
    };
    let (res, json) = match cli.command {
        None => (bare::run(cli.json), cli.json),
        Some(Command::Status(a)) => (status::run(&a), a.json),
        Some(Command::Ls(a)) => (query::ls(&a), a.json),
        Some(Command::Top(a)) => (query::top(&a), a.json),
        Some(Command::Find(a)) => (query::find(&a), a.json),
        Some(Command::Diff(a)) => (query::diff(&a), a.json),
        Some(Command::Scan(a)) => {
            let json = a.json;
            (scan::run(a), json)
        }
        Some(Command::Refresh(a)) => {
            let json = a.json;
            (refresh::run(a), json)
        }
        Some(Command::Schema(a)) => (schema::run(&a), true),
    };
    match res {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => output::fail(&e, json),
    }
}

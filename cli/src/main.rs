mod bare;
mod completions;
mod crash;
mod doctor;
mod install;
mod output;
mod query;
mod refresh;
mod scan;
mod schema;
mod serve;
mod skill;
mod status;

use std::process::ExitCode;

// The release archives are static musl builds; musl's allocator takes a lock
// per call, which made 8 walker threads slower than one (docs/benchmarks.md).
// The `override` feature replaces malloc for C code (opendir, SQLite) too.
#[cfg(target_env = "musl")]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use clap::{CommandFactory, FromArgMatches, Parser, Subcommand};

const HELP_TEMPLATE: &str = "\
{before-help}{name} {version} · {about}

{usage-heading} {usage}

Look:
  ls              Children of a folder, biggest first
  top             Largest files, or folders untouched for a while
  find            Everything matching a filter, with the total size
  diff            What grew or shrank since the previous scan
  gui             Browse the snapshot in your browser
  serve           The gui's local server, without opening a browser

Maintain:
  status          Snapshot age, free space, last error, running scan
  doctor          Check the timer, binary, access and space; print fixes
  scan            Scan and publish a new snapshot (root)
  refresh         Rescan one folder into the current snapshot (root)

Setup:
  install-timer   Nightly scan, read access for you, first scan now (root)
  skill           Install the Claude Code skill
  schema          JSON Schema of every --json answer
  completions     Shell completions for bash, zsh, fish

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
    /// Browse the snapshot in your browser
    #[command(after_help = "Examples:\n  freespace gui\n  freespace gui ~/Downloads --port 8710")]
    Gui(serve::GuiArgs),
    /// The gui's local server, without opening a browser
    #[command(after_help = "Examples:\n  freespace serve\n  freespace serve --port 8710")]
    Serve(serve::ServeArgs),
    /// Snapshot age, free space, last error, running scan
    #[command(after_help = "Examples:\n  freespace status\n  freespace status --json")]
    Status(status::StatusArgs),
    /// Check the timer, binary, access and space; print fixes
    #[command(after_help = "Examples:\n  freespace doctor\n  freespace doctor --json")]
    Doctor(doctor::DoctorArgs),
    /// Scan and publish a new snapshot (root)
    #[command(after_help = "Examples:\n  sudo freespace scan\n  sudo freespace scan / --workers 4")]
    Scan(scan::ScanArgs),
    /// Rescan one folder into the current snapshot (root)
    #[command(
        after_help = "Examples:\n  sudo freespace refresh /home/tass/.cache\n  sudo freespace refresh /var/log --json"
    )]
    Refresh(refresh::RefreshArgs),
    /// Nightly scan, read access for you, first scan now (root)
    #[command(
        name = "install-timer",
        after_help = "Examples:\n  sudo freespace install-timer\n  sudo freespace install-timer --roots / /home --at 03:30\n  sudo freespace install-timer --uninstall"
    )]
    InstallTimer(install::InstallArgs),
    /// Install the Claude Code skill
    #[command(
        after_help = "Examples:\n  freespace skill install\n  freespace skill print > SKILL.md"
    )]
    Skill(skill::SkillArgs),
    /// JSON Schema of every --json answer
    #[command(
        after_help = "Examples:\n  freespace schema --json > schema.json\n  freespace schema | jq '.commands'"
    )]
    Schema(schema::SchemaArgs),
    /// Shell completions for bash, zsh, fish
    #[command(
        after_help = "Examples:\n  freespace completions bash > ~/.local/share/bash-completion/completions/freespace\n  freespace completions fish > ~/.config/fish/completions/freespace.fish"
    )]
    Completions(completions::CompletionsArgs),
}

fn main() -> ExitCode {
    crash::install_hook();
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
        Some(Command::Gui(a)) => (serve::gui(&a).map_err(Into::into), false),
        Some(Command::Serve(a)) => (serve::run(&a).map_err(Into::into), false),
        Some(Command::Doctor(a)) => {
            return match doctor::run(&a) {
                Ok(true) => ExitCode::SUCCESS,
                Ok(false) => ExitCode::from(1),
                Err(error) => output::fail(&error, a.json),
            };
        }
        Some(Command::Scan(a)) => {
            let json = a.json;
            (scan::run(a), json)
        }
        Some(Command::Refresh(a)) => {
            let json = a.json;
            (refresh::run(a), json)
        }
        Some(Command::InstallTimer(a)) => {
            let json = a.json;
            (install::run(a), json)
        }
        Some(Command::Skill(a)) => (skill::run(a), false),
        Some(Command::Schema(a)) => (schema::run(&a), true),
        Some(Command::Completions(a)) => (completions::run(&a, &mut Cli::command()), false),
    };
    match res {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => output::fail(&e, json),
    }
}

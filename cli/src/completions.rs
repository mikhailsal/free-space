//! `completions bash|zsh|fish` (D19): install.sh writes them under
//! /usr/local/share; `install-timer --uninstall` removes them.

use clap::{Args, Command, ValueEnum};
use clap_complete::Shell;
use freespace_core::Result;

#[derive(Clone, Copy, ValueEnum)]
pub enum Sh {
    Bash,
    Zsh,
    Fish,
}

#[derive(Args)]
pub struct CompletionsArgs {
    pub shell: Sh,
}

pub fn run(a: &CompletionsArgs, cmd: &mut Command) -> Result<()> {
    let shell = match a.shell {
        Sh::Bash => Shell::Bash,
        Sh::Zsh => Shell::Zsh,
        Sh::Fish => Shell::Fish,
    };
    clap_complete::generate(shell, cmd, "freespace", &mut std::io::stdout());
    Ok(())
}

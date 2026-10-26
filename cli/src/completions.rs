//! `completions bash|zsh|fish` (D19): install.sh writes them under
//! /usr/local/share; `install-timer --uninstall` removes them.

use crate::output;
use clap::{Args, Command, ValueEnum};
use clap_complete::Shell;

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

pub fn run(a: &CompletionsArgs, cmd: &mut Command) -> output::Result<()> {
    let shell = match a.shell {
        Sh::Bash => Shell::Bash,
        Sh::Zsh => Shell::Zsh,
        Sh::Fish => Shell::Fish,
    };
    let mut bytes = Vec::new();
    clap_complete::generate(shell, cmd, "freespace", &mut bytes);
    output::print_bytes(&bytes)?;
    Ok(())
}

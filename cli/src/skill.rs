//! `skill install|print|uninstall` (D9, T43): the embedded SKILL.md and
//! schema.json, stamped with the binary version so doctor can spot a stale copy.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use clap::{Args, Subcommand};
use freespace_core::platform::{self, UserInfo};
use freespace_core::queries::types::schema_document;
use freespace_core::snapshot::fsutil;
use freespace_core::{Error, Result, VERSION};

use crate::output;

const SKILL_MD: &str = include_str!("../../skill/SKILL.md");
pub const FILES: [&str; 2] = ["SKILL.md", "schema.json"];
const STAMP: &str = "<!-- freespace-version: ";

#[derive(Args)]
pub struct SkillArgs {
    #[command(subcommand)]
    pub cmd: SkillCmd,
}

#[derive(Subcommand)]
pub enum SkillCmd {
    /// Write SKILL.md and schema.json to ~/.claude/skills/freespace (or --dir)
    Install {
        #[arg(long, value_hint = clap::ValueHint::DirPath)]
        dir: Option<PathBuf>,
    },
    /// Print SKILL.md to stdout
    Print,
    /// Remove the installed skill files
    Uninstall {
        #[arg(long, value_hint = clap::ValueHint::DirPath)]
        dir: Option<PathBuf>,
    },
}

/// SKILL.md with the version stamp after the front matter.
pub fn stamped() -> String {
    let stamp = format!("{STAMP}{VERSION} -->\n");
    match SKILL_MD
        .strip_prefix("---\n")
        .and_then(|r| r.find("\n---\n").map(|i| i + 4 + 5))
    {
        Some(end) => format!("{}{stamp}{}", &SKILL_MD[..end], &SKILL_MD[end..]),
        None => format!("{stamp}{SKILL_MD}"),
    }
}

/// Version stamped in an installed SKILL.md.
pub fn installed_version(dir: &Path) -> Option<String> {
    let text = fs::read_to_string(dir.join("SKILL.md")).ok()?;
    let rest = &text[text.find(STAMP)? + STAMP.len()..];
    Some(rest[..rest.find(" -->")?].to_owned())
}

pub fn default_dir_for(u: &UserInfo) -> PathBuf {
    u.home.join(".claude/skills/freespace")
}

/// `~/.claude/skills/freespace` of the human: `$SUDO_USER` under sudo.
fn target(dir: Option<PathBuf>) -> Result<(PathBuf, Option<UserInfo>)> {
    let user = platform::invoking_user();
    if let Some(d) = dir {
        return Ok((d, user.filter(|_| fsutil::is_root())));
    }
    match user {
        Some(u) => Ok((default_dir_for(&u), fsutil::is_root().then_some(u))),
        None => {
            let home = std::env::var_os("HOME")
                .ok_or_else(|| Error::usage("HOME is not set: pass --dir"))?;
            Ok((PathBuf::from(home).join(".claude/skills/freespace"), None))
        }
    }
}

pub fn run(a: SkillArgs) -> output::Result<()> {
    match a.cmd {
        SkillCmd::Print => {
            output::print(&stamped())?;
            Ok(())
        }
        SkillCmd::Install { dir } => {
            let (dir, owner) = target(dir)?;
            install(&dir, owner.as_ref())?;
            eprintln!("freespace skill {VERSION} installed in {}", dir.display());
            Ok(())
        }
        SkillCmd::Uninstall { dir } => {
            let (dir, _) = target(dir)?;
            let mut n = 0;
            for f in FILES {
                if fs::remove_file(dir.join(f)).is_ok() {
                    n += 1;
                }
            }
            let _ = fs::remove_dir(&dir);
            eprintln!("freespace skill: removed {n} files from {}", dir.display());
            Ok(())
        }
    }
}

/// Write both files (0644); when root acts for a user, hand them the files.
pub fn install(dir: &Path, owner: Option<&UserInfo>) -> Result<()> {
    fs::create_dir_all(dir).map_err(|e| Error::io(format!("create {}", dir.display()), e))?;
    let schema = serde_json::to_string_pretty(&schema_document()).unwrap_or_default() + "\n";
    for (name, body) in [("SKILL.md", stamped()), ("schema.json", schema)] {
        let p = dir.join(name);
        fs::write(&p, body).map_err(|e| Error::io(format!("write {}", p.display()), e))?;
        fs::set_permissions(&p, fs::Permissions::from_mode(0o644))
            .map_err(|e| Error::io(format!("chmod {}", p.display()), e))?;
    }
    if let Some(u) = owner {
        // chown the created chain inside the user's home: skills/freespace and above up to ~/.claude
        let mut paths: Vec<PathBuf> = FILES.iter().map(|f| dir.join(f)).collect();
        let mut d = Some(dir);
        while let Some(x) = d {
            if x == u.home || !x.starts_with(&u.home) {
                break;
            }
            paths.push(x.to_path_buf());
            d = x.parent();
        }
        for p in paths {
            std::os::unix::fs::chown(&p, Some(u.uid), Some(u.gid))
                .map_err(|e| Error::io(format!("chown {}", p.display()), e))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stamp_sits_after_front_matter_and_reads_back() {
        let s = stamped();
        assert!(s.starts_with("---\nname: freespace\n"));
        let fm_end = s[4..].find("\n---\n").unwrap() + 4 + 5;
        assert!(s[fm_end..].starts_with(STAMP), "{}", &s[..fm_end + 40]);
        let d = tempfile::tempdir().unwrap();
        install(d.path(), None).unwrap();
        assert_eq!(installed_version(d.path()).as_deref(), Some(VERSION));
        let schema: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(d.path().join("schema.json")).unwrap())
                .unwrap();
        assert!(schema["commands"]["ls"].is_object());
    }
}

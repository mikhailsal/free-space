//! `--help` describes every option, so nobody has to read the source to learn what `--limit`
//! or `--json` does. The walk starts at the top-level help and follows every command it lists.

use std::process::Command;

/// The commands a help text lists: two spaces, a lowercase word, two more spaces, then text.
fn commands(help: &str) -> Vec<String> {
    help.lines()
        .filter_map(|line| {
            let (name, tail) = line.strip_prefix("  ")?.split_once("  ")?;
            let word = !name.is_empty() && name.chars().all(|c| c.is_ascii_lowercase() || c == '-');
            (word && name != "help" && !tail.trim().is_empty()).then(|| name.to_owned())
        })
        .collect()
}

fn indent(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

/// Options that show no description: a flag in the flag column (6 spaces, or 2 before a short
/// flag) with nothing after it, and no deeper-indented line under it. A wrapped description
/// line also starts with a flag name sometimes, but it sits far deeper than the flag column.
fn blank_options(help: &str) -> Vec<String> {
    let lines: Vec<&str> = help.lines().collect();
    let mut blank = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let flag = match indent(line) {
            6 => line.trim().starts_with("--"),
            2 => line.trim().starts_with('-'),
            _ => false,
        };
        let described =
            line.trim().contains("  ") || lines.get(i + 1).is_some_and(|next| indent(next) > 6);
        if flag && !described {
            blank.push(line.trim().to_owned());
        }
    }
    blank
}

fn help_of(path: &[String]) -> String {
    let scratch = tempfile::tempdir().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_freespace"))
        .args(path)
        .arg("--help")
        .env("FREESPACE_CONFIG", scratch.path().join("none.toml"))
        .env("FREESPACE_SNAPSHOT_DIR", scratch.path().join("snap"))
        .output()
        .unwrap();
    assert!(out.status.success(), "{path:?} --help failed");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn walk(path: &[String], visited: &mut Vec<String>, blank: &mut Vec<String>) {
    visited.push(path.join(" "));
    let help = help_of(path);
    blank.extend(
        blank_options(&help)
            .into_iter()
            .map(|b| format!("freespace {} {b}", path.join(" "))),
    );
    for name in commands(&help) {
        let mut next = path.to_vec();
        next.push(name);
        walk(&next, visited, blank);
    }
}

#[test]
fn every_option_of_every_command_has_a_description() {
    let (mut visited, mut blank) = (Vec::new(), Vec::new());
    walk(&[], &mut visited, &mut blank);
    for known in [
        "ls",
        "top",
        "find",
        "diff",
        "status",
        "doctor",
        "scan",
        "install-timer",
        "skill install",
    ] {
        assert!(
            visited.iter().any(|v| v == known),
            "the walk never reached `{known}`: {visited:?}"
        );
    }
    assert!(
        blank.is_empty(),
        "options without a description in --help:\n{}",
        blank.join("\n")
    );
}

#[test]
fn the_check_finds_a_bare_option_and_ignores_wrapped_text() {
    let help = "Options:\n      --limit <LIMIT>\n      --json               Print it\n      --checkpoint         \
                Request one; this may do work beyond\n                           --budget\n  -h, --help           Print help\n";
    assert_eq!(blank_options(help), ["--limit <LIMIT>"]);
    let next_line = "Options:\n      --some <VALUE>\n          Described on the next line\n";
    assert!(blank_options(next_line).is_empty());
    assert_eq!(
        blank_options("Options:\n  -x\n      --y  Described\n"),
        ["-x"]
    );
}

#[test]
fn commands_are_read_from_grouped_and_plain_lists() {
    let help = "Look:\n  ls     Children\n  top    Largest\n\nCommands:\n  help  Print this\n  install  Write it\n\n\
                Examples:\n  freespace ls /\n  -h, --help   Print help\n      --json  Print it\n";
    assert_eq!(commands(help), ["ls", "top", "install"]);
}

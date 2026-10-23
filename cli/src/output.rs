//! Output conventions: answers on stdout (tables on a terminal or a pipe, JSON
//! under `--json`), progress and warnings on stderr; errors as text on stderr
//! plus the D16 body on stdout under `--json`.

use std::io::{IsTerminal, Write};
use std::process::ExitCode;

use freespace_core::config;
use freespace_core::queries::types::Header;
use freespace_core::{Error, ErrorCode};

pub fn print_json<T: serde::Serialize>(v: &T) {
    let mut out = std::io::stdout().lock();
    let _ = serde_json::to_writer_pretty(&mut out, v);
    let _ = writeln!(out);
}

pub fn stderr_is_tty() -> bool {
    std::io::stderr().is_terminal()
}

pub fn stdout_is_tty() -> bool {
    std::io::stdout().is_terminal()
}

pub fn fail(e: &Error, json: bool) -> ExitCode {
    eprintln!("freespace: {}", e.message);
    if let Some(fix) = &e.fix {
        eprintln!("fix: {fix}");
    }
    if json {
        print_error_body(e);
    }
    ExitCode::from(e.exit() as u8)
}

fn print_error_body(e: &Error) {
    let loaded = config::load().ok();
    let dir = loaded
        .as_ref()
        .map(|l| l.config.snapshot_dir.display().to_string());
    let cfg = loaded.as_ref().map(|l| l.path.display().to_string());
    print_json(&e.body(dir.as_deref(), cfg.as_deref()));
}

/// clap parse errors are USAGE (exit 2) and follow the same `--json` rule;
/// `--help` and `--version` print and exit 0.
pub fn clap_error(e: clap::Error) -> ExitCode {
    let _ = e.print();
    if !e.use_stderr() {
        return ExitCode::SUCCESS;
    }
    if std::env::args_os().any(|a| a == "--json") {
        print_error_body(
            &Error::new(ErrorCode::Usage, first_line(&e)).with_fix("freespace --help"),
        );
    }
    ExitCode::from(ErrorCode::Usage.exit() as u8)
}

fn first_line(e: &clap::Error) -> String {
    let s = e.render().to_string();
    s.lines()
        .next()
        .unwrap_or("bad arguments")
        .trim_start_matches("error: ")
        .to_owned()
}

/// D13: an incomplete snapshot is reported before any numbers.
pub fn warn_incomplete(h: &Header) {
    if !h.incomplete {
        return;
    }
    let n = h.unknown_subtrees.len();
    let list = h
        .unknown_subtrees
        .iter()
        .take(5)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    let more = if n > 5 {
        format!(" and {} more", n - 5)
    } else {
        String::new()
    };
    eprintln!("warning: incomplete snapshot, sizes under these folders are unknown: {list}{more}");
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Right,
}

/// Plain aligned columns, no color, the same on a terminal and in a pipe.
/// The last column is never padded so long paths do not leave trailing blanks.
pub struct Table {
    cols: Vec<(&'static str, Align)>,
    rows: Vec<Vec<String>>,
}

impl Table {
    pub fn new(cols: &[(&'static str, Align)]) -> Self {
        Table {
            cols: cols.to_vec(),
            rows: Vec::new(),
        }
    }

    pub fn row(&mut self, cells: Vec<String>) {
        self.rows.push(cells);
    }

    pub fn render(&self) -> String {
        let n = self.cols.len();
        let mut w: Vec<usize> = self.cols.iter().map(|(h, _)| width(h)).collect();
        for r in &self.rows {
            for (i, c) in r.iter().enumerate().take(n) {
                w[i] = w[i].max(width(c));
            }
        }
        let mut out = String::new();
        let line = |out: &mut String, cells: &[&str]| {
            let mut s = String::new();
            for (i, c) in cells.iter().enumerate() {
                if i > 0 {
                    s.push_str("  ");
                }
                let pad = w[i].saturating_sub(width(c));
                let last = i + 1 == cells.len();
                match self.cols[i].1 {
                    Align::Right => {
                        s.push_str(&" ".repeat(pad));
                        s.push_str(c);
                    }
                    Align::Left => {
                        s.push_str(c);
                        if !last {
                            s.push_str(&" ".repeat(pad));
                        }
                    }
                }
            }
            out.push_str(s.trim_end());
            out.push('\n');
        };
        let head: Vec<&str> = self.cols.iter().map(|(h, _)| *h).collect();
        line(&mut out, &head);
        for r in &self.rows {
            let cells: Vec<&str> = r.iter().map(String::as_str).collect();
            line(&mut out, &cells);
        }
        out
    }
}

fn width(s: &str) -> usize {
    s.chars().count()
}

/// Write to stdout, ignoring a closed pipe (`freespace ls / | head`).
pub fn print(s: &str) {
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(s.as_bytes());
    let _ = out.flush();
}

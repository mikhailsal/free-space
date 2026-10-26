//! Output conventions: answers on stdout (tables on a terminal or a pipe, JSON
//! under `--json`), progress and warnings on stderr; errors as text on stderr
//! plus the D16 body on stdout under `--json`.

use std::io::{IsTerminal, Write};
use std::process::ExitCode;

use freespace_core::config;
use freespace_core::queries::types::Header;
use freespace_core::{Error, ErrorCode};

/// Command failures keep stdout delivery separate from domain errors: after a
/// partial answer, reporting a second JSON document would corrupt the stream.
#[derive(Debug)]
pub enum CommandError {
    Core(Error),
    Output(std::io::Error),
    Serialize(serde_json::Error),
}

pub type Result<T> = std::result::Result<T, CommandError>;

impl From<Error> for CommandError {
    fn from(error: Error) -> Self {
        Self::Core(error)
    }
}

impl From<std::io::Error> for CommandError {
    fn from(error: std::io::Error) -> Self {
        Self::Output(error)
    }
}

fn ignore_broken_pipe(result: std::io::Result<()>) -> std::io::Result<()> {
    match result {
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
        other => other,
    }
}

pub fn print_json<T: serde::Serialize>(v: &T) -> Result<()> {
    write_json(&mut std::io::stdout().lock(), v)
}

fn write_json<W: Write, T: serde::Serialize>(out: &mut W, v: &T) -> Result<()> {
    if let Err(e) = serde_json::to_writer_pretty(&mut *out, v) {
        if e.io_error_kind() == Some(std::io::ErrorKind::BrokenPipe) {
            return Ok(());
        }
        return Err(CommandError::Serialize(e));
    }
    ignore_broken_pipe(writeln!(out).and_then(|()| out.flush()))?;
    Ok(())
}

pub fn stderr_is_tty() -> bool {
    std::io::stderr().is_terminal()
}

pub fn stdin_is_tty() -> bool {
    std::io::stdin().is_terminal()
}

pub fn stdout_is_tty() -> bool {
    std::io::stdout().is_terminal()
}

pub fn fail(error: &CommandError, json: bool) -> ExitCode {
    match error {
        CommandError::Core(e) => {
            eprintln!("freespace: {}", e.message);
            if let Some(fix) = &e.fix {
                eprintln!("fix: {fix}");
            }
            if json && let Err(error) = print_error_body(e) {
                return fail(&error, false);
            }
            ExitCode::from(e.exit() as u8)
        }
        CommandError::Output(e) => output_failure(e),
        CommandError::Serialize(e) => output_failure(e),
    }
}

fn output_failure(error: &dyn std::fmt::Display) -> ExitCode {
    eprintln!("freespace: write stdout: {error}");
    ExitCode::from(1)
}

fn print_error_body(e: &Error) -> Result<()> {
    let loaded = config::load().ok();
    let dir = loaded
        .as_ref()
        .map(|l| l.config.snapshot_dir.display().to_string());
    let cfg = loaded.as_ref().map(|l| l.path.display().to_string());
    print_json(&e.body(dir.as_deref(), cfg.as_deref()))
}

/// clap parse errors are USAGE (exit 2) and follow the same `--json` rule;
/// `--help` and `--version` print and exit 0 unless stdout delivery fails.
pub fn clap_error(e: clap::Error) -> ExitCode {
    if !e.use_stderr() {
        let result = e.print().and_then(|()| std::io::stdout().lock().flush());
        return match ignore_broken_pipe(result) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => output_failure(&error),
        };
    }
    let _ = e.print();
    if std::env::args_os().any(|a| a == "--json")
        && let Err(error) = print_error_body(
            &Error::new(ErrorCode::Usage, first_line(&e)).with_fix("freespace --help"),
        )
    {
        return fail(&error, false);
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
pub fn print(s: &str) -> std::io::Result<()> {
    print_bytes(s.as_bytes())
}

pub fn print_bytes(bytes: &[u8]) -> std::io::Result<()> {
    write_bytes(&mut std::io::stdout().lock(), bytes)
}

fn write_bytes(out: &mut impl Write, bytes: &[u8]) -> std::io::Result<()> {
    ignore_broken_pipe(out.write_all(bytes).and_then(|()| out.flush()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{self, ErrorKind};

    struct FaultWriter {
        remaining: Option<usize>,
        flush_error: bool,
        kind: ErrorKind,
        bytes: Vec<u8>,
    }

    impl Write for FaultWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            let n = match self.remaining {
                Some(0) => return Err(io::Error::from(self.kind)),
                Some(left) => left.min(bytes.len()),
                None => bytes.len(),
            };
            self.bytes.extend_from_slice(&bytes[..n]);
            if let Some(left) = &mut self.remaining {
                *left -= n;
            }
            Ok(n)
        }

        fn flush(&mut self) -> io::Result<()> {
            if self.flush_error {
                Err(io::Error::from(self.kind))
            } else {
                Ok(())
            }
        }
    }

    #[test]
    fn writes_and_flushes_ignore_only_broken_pipe() {
        for kind in [ErrorKind::BrokenPipe, ErrorKind::StorageFull] {
            for flush_error in [false, true] {
                for json in [false, true] {
                    let mut out = FaultWriter {
                        remaining: if flush_error { None } else { Some(2) },
                        flush_error,
                        kind,
                        bytes: Vec::new(),
                    };
                    let result = if json {
                        write_json(&mut out, &vec!["payload"])
                    } else {
                        write_bytes(&mut out, b"payload").map_err(CommandError::from)
                    };
                    assert_eq!(result.is_ok(), kind == ErrorKind::BrokenPipe);
                    if !flush_error {
                        assert_eq!(out.bytes.len(), 2, "short write was attempted");
                    }
                }
            }
        }
    }

    #[test]
    fn serialization_failure_stops_without_a_newline() {
        struct Unserializable;
        impl serde::Serialize for Unserializable {
            fn serialize<S: serde::Serializer>(
                &self,
                serializer: S,
            ) -> std::result::Result<S::Ok, S::Error> {
                use serde::ser::SerializeSeq;
                let mut sequence = serializer.serialize_seq(Some(2))?;
                sequence.serialize_element("partial")?;
                Err(serde::ser::Error::custom(
                    "intentional serialization failure",
                ))
            }
        }
        let mut bytes = Vec::new();
        assert!(matches!(
            write_json(&mut bytes, &Unserializable),
            Err(CommandError::Serialize(_))
        ));
        assert_eq!(bytes, b"[\n  \"partial\"");
    }
}

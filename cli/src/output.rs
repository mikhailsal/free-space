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

/// Escape terminal control characters in dynamic display data only. Keep Unicode
/// names readable, and leave machine responses and intentional layout untouched.
pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if c.is_control() {
            out.extend(c.escape_default());
        } else {
            out.push(c);
        }
    }
    out
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
            eprintln!("freespace: {}", escape(&e.message));
            if let Some(fix) = &e.fix {
                eprintln!("fix: {}", escape(fix));
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
    eprintln!("freespace: write stdout: {}", escape(&error.to_string()));
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
pub fn clap_error(mut e: clap::Error) -> ExitCode {
    if !e.use_stderr() {
        let result = e.print().and_then(|()| std::io::stdout().lock().flush());
        return match ignore_broken_pipe(result) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => output_failure(&error),
        };
    }
    let message = first_line(&e);
    escape_clap_values(&mut e);
    let _ = e.print();
    if std::env::args_os().any(|a| a == "--json")
        && let Err(error) =
            print_error_body(&Error::new(ErrorCode::Usage, message).with_fix("freespace --help"))
    {
        return fail(&error, false);
    }
    ExitCode::from(ErrorCode::Usage.exit() as u8)
}

// Clap owns the diagnostic layout and styling; its string contexts hold the
// argument data. Preserve the original message for the machine error body.
fn escape_clap_values(e: &mut clap::Error) {
    use clap::error::{ContextKind, ContextValue};
    let unsafe_value = e.context().any(|(_, value)| match value {
        ContextValue::String(s) => s.chars().any(char::is_control),
        ContextValue::Strings(ss) => ss.iter().any(|s| s.chars().any(char::is_control)),
        _ => false,
    });
    // Rich suggestions embed the rejected value in an opaque styled string.
    // Clap exposes no span-level editing API. Omit those optional tips for
    // control-bearing input, keeping the escaped main diagnostic and usage.
    if unsafe_value {
        e.remove(ContextKind::Suggested);
    }
    let values: Vec<_> = e
        .context()
        .filter_map(|(kind, value)| {
            let value = match value {
                ContextValue::String(s) => ContextValue::String(escape(s)),
                ContextValue::Strings(ss) => {
                    ContextValue::Strings(ss.iter().map(|s| escape(s)).collect())
                }
                _ => return None,
            };
            Some((kind, value))
        })
        .collect();
    for (kind, value) in values {
        e.insert(kind, value);
    }
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
        .map(|path| escape(path))
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
        self.rows
            .push(cells.iter().map(|cell| escape(cell)).collect());
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
        let head: Vec<&str> = self.cols.iter().map(|(h, _)| *h).collect();
        self.render_line(&mut out, &head, &w);
        for r in &self.rows {
            let cells: Vec<&str> = r.iter().map(String::as_str).collect();
            self.render_line(&mut out, &cells, &w);
        }
        out
    }

    fn render_line(&self, out: &mut String, cells: &[&str], widths: &[usize]) {
        let mut line = String::new();
        for (i, cell) in cells.iter().enumerate() {
            if i > 0 {
                line.push_str("  ");
            }
            let pad = widths[i].saturating_sub(width(cell));
            let last = i + 1 == cells.len();
            self.render_cell(&mut line, i, cell, pad, last);
        }
        out.push_str(line.trim_end());
        out.push('\n');
    }

    fn render_cell(&self, line: &mut String, i: usize, cell: &str, pad: usize, last: bool) {
        match self.cols[i].1 {
            Align::Right => {
                line.push_str(&" ".repeat(pad));
                line.push_str(cell);
            }
            Align::Left => {
                line.push_str(cell);
                if !last {
                    line.push_str(&" ".repeat(pad));
                }
            }
        }
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

    #[test]
    fn escapes_all_terminal_controls_but_preserves_readable_unicode() {
        let readable = "имя 😀 café /back\\slash";
        assert_eq!(escape(readable), readable);
        // Display escapes are terminal-safe, not a reversible filename encoding.
        assert_eq!(escape("\n"), r"\n");
        assert_eq!(escape(r"\n"), r"\n");
        for value in (0..=0x1f).chain(0x7f..=0x9f) {
            let escaped = escape(&char::from_u32(value).unwrap().to_string());
            assert!(escaped.starts_with('\\'));
            assert!(!escaped.chars().any(char::is_control));
        }
    }

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
        for (kind, flush_error, json) in [
            (ErrorKind::BrokenPipe, false, false),
            (ErrorKind::BrokenPipe, false, true),
            (ErrorKind::BrokenPipe, true, false),
            (ErrorKind::BrokenPipe, true, true),
            (ErrorKind::StorageFull, false, false),
            (ErrorKind::StorageFull, false, true),
            (ErrorKind::StorageFull, true, false),
            (ErrorKind::StorageFull, true, true),
        ] {
            assert_write_behavior(kind, flush_error, json);
        }
    }

    fn assert_write_behavior(kind: ErrorKind, flush_error: bool, json: bool) {
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

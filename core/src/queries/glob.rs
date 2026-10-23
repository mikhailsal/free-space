//! `fs_glob(name, pattern, icase)`: glob over entry names (T17).
//!
//! The glob is translated to `regex::bytes` patterns so names that are not
//! valid UTF-8 still match: `*` = any bytes, `?` = one character in a UTF-8
//! name and one byte in a name that is not UTF-8 (two compiled regexes),
//! `[..]`/`[!..]` classes, `\` escapes. `icase` compiles with `(?i)` and
//! Unicode case folding.

use std::sync::Arc;

use regex::bytes::{Regex, RegexBuilder};
use rusqlite::Connection;
use rusqlite::functions::FunctionFlags;
use rusqlite::types::ValueRef;

use crate::error::{Error, Result};

/// `bytewise`: `?` matches one byte (for names that are not UTF-8).
pub fn glob_to_regex(glob: &str, bytewise: bool) -> std::result::Result<String, String> {
    let mut re = String::from("^");
    let chars: Vec<char> = glob.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '*' => re.push_str("(?s-u:.)*"),
            '?' if bytewise => re.push_str("(?s-u:.)"),
            '?' => re.push_str("(?s:.)"),
            '\\' => {
                i += 1;
                match chars.get(i) {
                    Some(&n) => re.push_str(&regex::escape(&n.to_string())),
                    None => return Err("pattern ends with a lone backslash".into()),
                }
            }
            '[' => {
                let mut j = i + 1;
                let mut class = String::from("[");
                if matches!(chars.get(j), Some('!') | Some('^')) {
                    class.push('^');
                    j += 1;
                }
                let start = j;
                let mut closed = false;
                while j < chars.len() {
                    let k = chars[j];
                    if k == ']' && j > start {
                        closed = true;
                        break;
                    }
                    match k {
                        '\\' => {
                            j += 1;
                            if let Some(&n) = chars.get(j) {
                                class.push_str(&regex::escape(&n.to_string()));
                            }
                        }
                        // '-' stays a range operator, but "--" would be set difference
                        '-' if !class.ends_with('-') => class.push('-'),
                        '[' | ']' | '^' | '&' | '~' | '-' => {
                            class.push('\\');
                            class.push(k);
                        }
                        _ => class.push(k),
                    }
                    j += 1;
                }
                if closed {
                    class.push(']');
                    re.push_str(&class);
                    i = j;
                } else {
                    // unmatched '[' is a literal
                    re.push_str(r"\[");
                }
            }
            _ => re.push_str(&regex::escape(&c.to_string())),
        }
        i += 1;
    }
    re.push('$');
    Ok(re)
}

/// A compiled pattern: one regex for UTF-8 names, one for the rest.
pub struct Matcher {
    utf8: Regex,
    bytes: Regex,
}

impl Matcher {
    pub fn is_match(&self, name: &[u8]) -> bool {
        if std::str::from_utf8(name).is_ok() {
            self.utf8.is_match(name)
        } else {
            self.bytes.is_match(name)
        }
    }
}

fn build(glob: &str, icase: bool, bytewise: bool) -> Result<Regex> {
    let bad = |e: String| Error::usage(format!("bad --name pattern {glob:?}: {e}"));
    let re = glob_to_regex(glob, bytewise).map_err(bad)?;
    RegexBuilder::new(&re)
        .case_insensitive(icase)
        .unicode(true)
        .size_limit(1 << 22)
        .build()
        .map_err(|e| bad(e.to_string()))
}

pub fn compile(glob: &str, icase: bool) -> Result<Matcher> {
    Ok(Matcher {
        utf8: build(glob, icase, false)?,
        bytes: build(glob, icase, true)?,
    })
}

/// Validate a pattern before running a query (usage error, exit 2).
pub fn validate(glob: &str) -> Result<()> {
    compile(glob, false).map(|_| ())
}

/// Register on every connection (the factory in `snapshot::reader::open_file`).
pub fn register(conn: &Connection) -> Result<()> {
    conn.create_scalar_function(
        "fs_glob",
        3,
        FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC,
        |ctx| {
            let icase: bool = ctx.get::<i64>(2).unwrap_or(0) != 0;
            let re: Arc<Matcher> = ctx.get_or_create_aux(1, |v: ValueRef<'_>| -> std::result::Result<Matcher, Box<dyn std::error::Error + Send + Sync>> {
                let p = v.as_str().map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)?;
                compile(p, icase).map_err(|e| e.message.clone().into())
            })?;
            let name: &[u8] = match ctx.get_raw(0) {
                ValueRef::Blob(b) => b,
                ValueRef::Text(t) => t,
                _ => return Ok(false),
            };
            Ok(re.is_match(name))
        },
    )
    .map_err(Error::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(glob: &str, icase: bool, name: &[u8]) -> bool {
        compile(glob, icase).unwrap().is_match(name)
    }

    #[test]
    fn basics() {
        assert!(m("daily*.mp4", false, b"daily1.mp4"));
        assert!(!m("daily*.mp4", false, b"Daily1.mp4"));
        assert!(m("daily*.mp4", true, b"Daily1.MP4"));
        assert!(m("*.iso", false, b".iso"));
        assert!(m("a?c", false, b"abc"));
        assert!(!m("a?c", false, b"ac"));
        assert!(m("node_modules", false, b"node_modules"));
        assert!(!m("node_modules", false, b"node_modules2"));
    }

    #[test]
    fn unicode_case_folding() {
        assert!(m("*видео*", true, "Видео.mkv".as_bytes()));
        assert!(!m("*видео*", false, "Видео.mkv".as_bytes()));
        assert!(m("?идео.mkv", false, "Видео.mkv".as_bytes()));
    }

    #[test]
    fn question_mark_is_one_character() {
        assert!(m("??", false, b"ab"));
        assert!(!m("??", false, "é".as_bytes()));
        assert!(m("??", false, "日本".as_bytes()));
        assert!(!m("??????", false, "日本".as_bytes()));
        assert!(m("?", false, "é".as_bytes()));
    }

    #[test]
    fn escapes_and_classes() {
        assert!(m(r"\[x]", false, b"[x]"));
        assert!(m(r"\*", false, b"*"));
        assert!(!m(r"\*", false, b"a"));
        assert!(m("[abc].txt", false, b"b.txt"));
        assert!(!m("[!abc].txt", false, b"b.txt"));
        assert!(m("[a-c]x", false, b"bx"));
        assert!(m("[", false, b"["));
        assert!(m("a.b", false, b"a.b"));
        assert!(!m("a.b", false, b"axb"));
        assert!(compile("x\\", false).is_err());
    }

    #[test]
    fn non_utf8_names() {
        let name = b"bad\xff\xfename.log";
        assert!(m("*.log", false, name));
        assert!(m("bad??name.log", false, name));
        assert!(m("bad*", true, name));
    }

    #[test]
    fn sql_function() {
        let c = Connection::open_in_memory().unwrap();
        register(&c).unwrap();
        let hit: bool = c
            .query_row(
                "SELECT fs_glob(CAST('Видео.mkv' AS BLOB), '*видео*', 1)",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(hit);
        let bare = Connection::open_in_memory().unwrap();
        assert!(
            bare.query_row("SELECT fs_glob('a', 'a', 0)", [], |r| r.get::<_, bool>(0))
                .is_err()
        );
    }
}

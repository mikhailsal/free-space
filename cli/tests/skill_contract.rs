//! T23 / D17: every `freespace …` command in skill/SKILL.md and README.md runs
//! here. Commands with `--json` run on a synthetic snapshot, must exit 0 and
//! must validate against skill/schema.json. The rest (sudo, gui, serve, ssh
//! lines, and every block after `<!-- no-run -->` in the README) are parsed by
//! clap only, with `--help` appended. A renamed or bogus flag fails either way.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime};

use serde_json::{Value, json};

struct Doc {
    at: String,
    text: String,
    /// false: parse with clap only
    run: bool,
}

/// Commands in a Markdown file: lines of fenced blocks and inline code spans
/// that call `freespace`.
fn commands(file: &str, md: &str) -> Vec<Doc> {
    let mut out = Vec::new();
    let (mut fenced, mut no_run, mut block_no_run) = (false, false, false);
    for (i, line) in md.lines().enumerate() {
        let at = format!("{file}:{}", i + 1);
        let t = line.trim();
        if t.starts_with("<!-- no-run -->") {
            no_run = true;
            continue;
        }
        if t.starts_with("```") {
            fenced = !fenced;
            if fenced {
                block_no_run = no_run;
                no_run = false;
            }
            continue;
        }
        let spans: Vec<&str> = if fenced {
            vec![t.strip_prefix("$ ").unwrap_or(t)]
        } else {
            line.split('`').skip(1).step_by(2).collect()
        };
        for s in spans {
            let words = shell_words(s);
            let Some(k) = words.iter().position(|w| w == "freespace") else {
                continue;
            };
            let run = !(fenced && block_no_run) && k == 0 && words.iter().any(|w| w == "--json");
            out.push(Doc {
                at: at.clone(),
                text: s.to_owned(),
                run,
            });
        }
    }
    out
}

/// Split like a POSIX shell (quotes, no expansion), stopping at a pipe,
/// redirect or command separator.
fn shell_words(s: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut cur: Option<String> = None;
    let mut quote: Option<char> = None;
    for c in s.chars() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), c) => cur.get_or_insert_default().push(c),
            (None, '\'' | '"') => {
                quote = Some(c);
                cur.get_or_insert_default();
            }
            (None, '|' | '>' | ';' | '&') if cur.is_none() => break,
            (None, c) if c.is_whitespace() => words.extend(cur.take()),
            (None, c) => cur.get_or_insert_default().push(c),
        }
    }
    words.extend(cur);
    words
}

struct Fixture {
    tmp: tempfile::TempDir,
    /// The fixture's `/`.
    root: String,
    schema: Value,
}

impl Fixture {
    /// A small `/` with the folders the examples name, scanned twice so
    /// `diff` has a previous generation.
    fn new() -> Fixture {
        let tmp = tempfile::tempdir().unwrap();
        let f = Fixture {
            root: tmp.path().join("r").to_str().unwrap().to_owned(),
            tmp,
            schema: serde_json::from_str(
                &fs::read_to_string(
                    Path::new(env!("CARGO_MANIFEST_DIR")).join("../skill/schema.json"),
                )
                .unwrap(),
            )
            .unwrap(),
        };
        let r = PathBuf::from(&f.root);
        let old = SystemTime::now() - Duration::from_secs(400 * 86_400);
        for (p, kb, aged) in [
            ("home/u/Downloads/ubuntu.iso", 600, false),
            ("home/u/proj/node_modules/pkg/index.js", 40, false),
            ("home/u/old/archive.tar", 300, true),
            ("home/u/.cache/pip/wheel", 200, false),
            ("var/cache/apt/pkg.deb", 100, true),
        ] {
            let p = r.join(p);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(&p, vec![1u8; kb << 10]).unwrap();
            if aged {
                fs::File::open(&p).unwrap().set_modified(old).unwrap();
            }
        }
        f.ok(&["scan", &f.root]);
        fs::write(r.join("home/u/Downloads/new.mp4"), vec![1u8; 900 << 10]).unwrap();
        f.ok(&["scan", &f.root]);
        f
    }

    fn cmd(&self, args: &[String]) -> Command {
        let mut c = Command::new(env!("CARGO_BIN_EXE_freespace"));
        c.args(args)
            .env("FREESPACE_CONFIG", self.tmp.path().join("none.toml"))
            .env("FREESPACE_SNAPSHOT_DIR", self.tmp.path().join("snap"))
            .env("HOME", self.tmp.path());
        c
    }

    fn ok(&self, args: &[&str]) {
        let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
        let o = self.cmd(&args).output().unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    }

    /// Arguments after `freespace`, with `<placeholders>` filled in and
    /// absolute paths moved under the fixture root.
    fn args(&self, text: &str) -> Vec<String> {
        let mut filled = String::new();
        let mut rest = text;
        while let Some(a) = rest.find('<') {
            let Some(b) = rest[a..].find('>') else { break };
            filled.push_str(&rest[..a]);
            let hole = &rest[a + 1..a + b];
            filled.push_str(if hole == "hex" {
                "2f"
            } else if hole == "user" {
                "u"
            } else if hole.contains(' ') || hole.chars().all(|c| c.is_ascii_lowercase()) {
                "/"
            } else {
                &rest[a..=a + b]
            });
            rest = &rest[a + b + 1..];
        }
        filled.push_str(rest);
        let words = shell_words(&filled);
        let k = words.iter().position(|w| w == "freespace").unwrap();
        let root = &self.root;
        words[k + 1..]
            .iter()
            .map(|w| match w.strip_prefix('/') {
                Some("") => root.to_owned(),
                Some(p) => format!("{root}/{p}"),
                None => w.clone(),
            })
            .collect()
    }

    fn check(&self, d: &Doc) -> Result<(), String> {
        let mut args = self.args(&d.text);
        if !d.run {
            args.push("--help".into());
            let o = self.cmd(&args).output().unwrap();
            return match o.status.code() {
                Some(0) => Ok(()),
                c => Err(format!(
                    "clap rejects it (exit {c:?}): {}",
                    String::from_utf8_lossy(&o.stderr).trim()
                )),
            };
        }
        let o = self.cmd(&args).output().unwrap();
        let code = o.status.code().unwrap_or(-1);
        let v: Value = serde_json::from_slice(&o.stdout).map_err(|e| {
            format!(
                "stdout is not JSON ({e}): {}",
                String::from_utf8_lossy(&o.stdout)
            )
        })?;
        let sub = args
            .iter()
            .find(|a| !a.starts_with('-'))
            .map(String::as_str);
        let answer = match sub {
            None => Some("status"),
            Some(c @ ("status" | "ls" | "top" | "find" | "diff" | "doctor")) => Some(c),
            Some(_) => None,
        };
        // doctor exits 1 when a check fails, which it does off a real install
        let allowed = if answer == Some("doctor") {
            vec![0, 1]
        } else {
            vec![0]
        };
        if !allowed.contains(&code) {
            return Err(format!("exit {code}: {v}"));
        }
        if let Some(name) = answer {
            self.validate(name, &v)?;
        }
        Ok(())
    }

    fn validate(&self, command: &str, v: &Value) -> Result<(), String> {
        let r = &self.schema["commands"][command]["$ref"];
        let schema = json!({
            "$schema": self.schema["$schema"],
            "$defs": self.schema["$defs"],
            "$ref": r,
        });
        let validator = jsonschema::validator_for(&schema).map_err(|e| e.to_string())?;
        let errs: Vec<String> = validator
            .iter_errors(v)
            .map(|e| format!("{} at {}", e, e.instance_path()))
            .collect();
        if errs.is_empty() {
            Ok(())
        } else {
            Err(format!("does not match {r}: {}", errs.join("; ")))
        }
    }
}

fn docs() -> Vec<Doc> {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut out = Vec::new();
    for file in ["skill/SKILL.md", "README.md"] {
        if let Ok(md) = fs::read_to_string(base.join(file)) {
            out.extend(commands(file, &md));
        }
    }
    out
}

#[test]
fn every_documented_command_works_and_answers_the_schema() {
    let f = Fixture::new();
    let docs = docs();
    let ran = docs.iter().filter(|d| d.run).count();
    assert!(ran >= 8, "only {ran} runnable commands found");
    let failures: Vec<String> = docs
        .iter()
        .filter_map(|d| {
            f.check(d)
                .err()
                .map(|e| format!("{} `{}`: {e}", d.at, d.text))
        })
        .collect();
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

#[test]
fn a_bogus_flag_in_an_example_fails_the_contract() {
    let f = Fixture::new();
    let doc = |text: &str, run| Doc {
        at: "test".into(),
        text: text.into(),
        run,
    };
    assert!(f.check(&doc("freespace ls / --json", true)).is_ok());
    assert!(
        f.check(&doc("freespace ls / --bogus --json", true))
            .is_err()
    );
    assert!(
        f.check(&doc("freespace top / --older-than soon --json", true))
            .is_err()
    );
    assert!(
        f.check(&doc("sudo freespace refresh <path>", false))
            .is_ok()
    );
    assert!(
        f.check(&doc("sudo freespace refresh <path> --bogus", false))
            .is_err()
    );
    assert!(f.check(&doc("freespace gui --prot 8710", false)).is_err());
}

#[test]
fn commands_are_found_in_spans_blocks_and_no_run_blocks() {
    let md = "Run `freespace ls / --json` or `sudo freespace scan`.\n\
              ```\n$ freespace top / --json | jq .\n```\n\
              <!-- no-run -->\n```sh\nfreespace status --json\n```\n";
    let d = commands("x.md", md);
    let got: Vec<(&str, bool)> = d.iter().map(|d| (d.text.as_str(), d.run)).collect();
    assert_eq!(
        got,
        [
            ("freespace ls / --json", true),
            ("sudo freespace scan", false),
            ("freespace top / --json | jq .", true),
            ("freespace status --json", false),
        ]
    );
    assert_eq!(
        shell_words("freespace find / --name '*.iso' --json | jq"),
        ["freespace", "find", "/", "--name", "*.iso", "--json"]
    );
}

//! `/etc/freespace/config.toml`. Missing file → defaults; unknown keys and
//! bad values → CONFIG (exit 2) naming the key.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, ErrorCode, Result};

pub const DEFAULT_CONFIG: &str = "/etc/freespace/config.toml";
pub const DEFAULT_SNAPSHOT_DIR: &str = "/var/lib/freespace";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Backend {
    #[default]
    V1,
    V2,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Experimental generation store, explicitly opt in with v2.
    pub backend: Backend,
    /// Empty: real filesystems from /proc/self/mountinfo.
    pub roots: Vec<PathBuf>,
    pub snapshot_dir: PathBuf,
    pub keep_previous: bool,
    /// Walker threads; unset = min(CPUs, 8).
    pub workers: Option<usize>,
    /// Use disk-backed staging for full scans.
    pub low_memory: bool,
    /// `serve`/`gui` port; 0 = random.
    pub port: u16,
    pub interval: String,
    pub at: String,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            backend: Backend::V1,
            roots: Vec::new(),
            snapshot_dir: PathBuf::from(DEFAULT_SNAPSHOT_DIR),
            keep_previous: true,
            workers: None,
            low_memory: false,
            port: 0,
            interval: "daily".into(),
            at: "02:00".into(),
        }
    }
}

/// `FREESPACE_CONFIG` overrides the path (tests, non-root development).
pub fn config_path() -> PathBuf {
    std::env::var_os("FREESPACE_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_CONFIG))
}

#[derive(Debug, Clone)]
pub struct Loaded {
    pub config: Config,
    pub path: PathBuf,
    pub exists: bool,
    /// A saved choice, distinct from a missing key that uses Config's default.
    pub configured_low_memory: Option<bool>,
}

impl Loaded {
    pub fn snapshot_dir(&self) -> crate::snapshot::SnapshotDir {
        crate::snapshot::SnapshotDir::new(&self.config.snapshot_dir)
    }
}

pub fn parse(text: &str, path: &Path) -> Result<Config> {
    parse_document(text, path).map(|(config, _)| config)
}

fn parse_document(text: &str, path: &Path) -> Result<(Config, Option<bool>)> {
    let error = |e: toml::de::Error| {
        Error::new(
            ErrorCode::Config,
            format!("{}: {}", path.display(), e.message()),
        )
        .with_fix(format!("fix or remove the key in {}", path.display()))
    };
    // Inspect presence in the same text, without another filesystem read.
    let document: toml::Value = toml::from_str(text).map_err(error)?;
    let configured_low_memory = document
        .get("low_memory")
        .map(|value| {
            value.as_bool().ok_or_else(|| {
                Error::new(
                    ErrorCode::Config,
                    format!("{}: low_memory must be true or false", path.display()),
                )
                .with_fix(format!("fix low_memory in {}", path.display()))
            })
        })
        .transpose()?;
    let config: Config = toml::from_str(text).map_err(error)?;
    validate(&config, path)?;
    Ok((config, configured_low_memory))
}

fn validate(c: &Config, path: &Path) -> Result<()> {
    let bad = |key: &str, why: &str| {
        Err(Error::new(
            ErrorCode::Config,
            format!("{}: {key}: {why}", path.display()),
        )
        .with_fix(format!("fix {key} in {}", path.display())))
    };
    for r in &c.roots {
        if !r.is_absolute() {
            return bad("roots", &format!("{} is not an absolute path", r.display()));
        }
    }
    if !c.snapshot_dir.is_absolute() {
        return bad("snapshot_dir", "must be an absolute path");
    }
    if c.workers == Some(0) {
        return bad("workers", "must be at least 1");
    }
    if !matches!(c.interval.as_str(), "hourly" | "daily" | "weekly") {
        return bad("interval", "must be hourly, daily or weekly");
    }
    let hm: Vec<&str> = c.at.split(':').collect();
    let ok = hm.len() == 2
        && hm[0].parse::<u8>().is_ok_and(|h| h < 24)
        && hm[1].parse::<u8>().is_ok_and(|m| m < 60)
        && hm[1].len() == 2;
    if !ok {
        return bad("at", "must be HH:MM");
    }
    Ok(())
}

pub fn load() -> Result<Loaded> {
    load_from(&config_path())
}

pub fn load_from(path: &Path) -> Result<Loaded> {
    let (mut config, exists, configured_low_memory) = match std::fs::read_to_string(path) {
        Ok(t) => {
            let (config, configured_low_memory) = parse_document(&t, path)?;
            (config, true, configured_low_memory)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (Config::default(), false, None),
        Err(e) => {
            return Err(Error::new(
                ErrorCode::Config,
                format!("cannot read {}: {e}", path.display()),
            ));
        }
    };
    if let Some(d) = std::env::var_os("FREESPACE_SNAPSHOT_DIR") {
        config.snapshot_dir = PathBuf::from(d);
    }
    Ok(Loaded {
        config,
        path: path.to_path_buf(),
        exists,
        configured_low_memory,
    })
}

/// Render the config file `install-timer` writes.
pub fn render(c: &Config) -> String {
    let roots: Vec<String> = c
        .roots
        .iter()
        .map(|r| format!("{:?}", r.display().to_string()))
        .collect();
    let mut s = String::from("# freespace configuration (written by `freespace install-timer`)\n");
    s.push_str(&format!("roots = [{}]\n", roots.join(", ")));
    s.push_str(&format!(
        "snapshot_dir = {:?}\n",
        c.snapshot_dir.display().to_string()
    ));
    if c.backend == Backend::V2 {
        s.push_str("backend = \"v2\"\n");
    }
    s.push_str(&format!("keep_previous = {}\n", c.keep_previous));
    if let Some(w) = c.workers {
        s.push_str(&format!("workers = {w}\n"));
    }
    s.push_str(&format!("low_memory = {}\n", c.low_memory));
    s.push_str(&format!("port = {}\n", c.port));
    s.push_str(&format!("interval = {:?}\n", c.interval));
    s.push_str(&format!("at = {:?}\n", c.at));
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_roundtrip() {
        let p = Path::new("/etc/freespace/config.toml");
        assert_eq!(parse("", p).unwrap(), Config::default());
        let c = Config {
            roots: vec!["/".into(), "/home".into()],
            workers: Some(4),
            low_memory: true,
            backend: Backend::V2,
            ..Default::default()
        };
        assert_eq!(parse(&render(&c), p).unwrap(), c);
    }

    #[test]
    fn errors_name_the_key() {
        let p = Path::new("/etc/freespace/config.toml");
        let e = parse("rootz = []", p).unwrap_err();
        assert_eq!(e.code, ErrorCode::Config);
        assert!(e.message.contains("rootz"), "{}", e.message);
        let e = parse("backend = \"v3\"", p).unwrap_err();
        assert!(e.message.contains("v3"));
        let e = parse("at = \"25:00\"", p).unwrap_err();
        assert!(e.message.contains("at"));
        let e = parse("roots = [\"relative\"]", p).unwrap_err();
        assert!(e.message.contains("roots"));
        let e = parse("low_memory = \"auto\"", p).unwrap_err();
        assert!(e.message.contains("low_memory"));
    }

    #[test]
    fn loaded_presence_distinguishes_missing_false_and_true() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.toml");
        let missing = load_from(&path).unwrap();
        assert!(!missing.exists);
        assert_eq!(missing.configured_low_memory, None);
        for (text, expected) in [
            ("", None),
            ("low_memory = false", Some(false)),
            ("low_memory = true", Some(true)),
        ] {
            std::fs::write(&path, text).unwrap();
            let loaded = load_from(&path).unwrap();
            assert!(loaded.exists);
            assert_eq!(loaded.configured_low_memory, expected);
            assert_eq!(loaded.config.low_memory, expected.unwrap_or(false));
            let rendered = render(&loaded.config);
            assert!(rendered.contains(&format!("low_memory = {}\n", loaded.config.low_memory)));
            std::fs::write(&path, rendered).unwrap();
            assert_eq!(
                load_from(&path).unwrap().configured_low_memory,
                Some(loaded.config.low_memory)
            );
        }
    }
}

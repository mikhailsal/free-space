//! Shared temporary paths and isolated commands for CLI integration tests.

use std::path::PathBuf;
use std::process::{Command, Output};

pub struct Env {
    pub tmp: tempfile::TempDir,
}

impl Env {
    pub fn new() -> Self {
        Self {
            tmp: tempfile::tempdir().unwrap(),
        }
    }

    pub fn root(&self) -> PathBuf {
        self.tmp.path().join("r")
    }

    pub fn snap(&self) -> PathBuf {
        self.tmp.path().join("snap")
    }

    pub fn config(&self) -> PathBuf {
        self.tmp.path().join("none.toml")
    }

    pub fn cmd(&self, args: &[&str]) -> Command {
        let mut c = Command::new(env!("CARGO_BIN_EXE_freespace"));
        c.args(args)
            .env("XDG_STATE_HOME", self.tmp.path().join("state"))
            .env("FREESPACE_CONFIG", self.config())
            .env("FREESPACE_SNAPSHOT_DIR", self.snap())
            .env_remove("FREESPACE_FAULT")
            .env_remove("FREESPACE_PAUSE");
        c
    }

    pub fn run(&self, args: &[&str]) -> Output {
        self.cmd(args).output().unwrap()
    }
}

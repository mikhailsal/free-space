//! Error contract (D16): every rescued error carries a stable `code`, its exit
//! number, a one-line `fix` and a machine-readable `recovery` hint. The CLI
//! prints the text on stderr and, under `--json`, the [`ErrorBody`] on stdout;
//! `/api/*` returns the same body with an HTTP status.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    /// Unexpected failure (I/O, SQLite, publish step).
    Generic,
    /// Not enough free space for a new snapshot (precheck or reserve).
    Space,
    /// Bad command line or query parameter.
    Usage,
    /// `config.toml` unreadable or invalid.
    Config,
    /// `gui` without a terminal on stdin and stdout.
    GuiNoTty,
    /// The path is not in the snapshot.
    PathNotFound,
    /// `diff` without a previous generation.
    NoPrev,
    /// The snapshot rotated between opening current and prev.
    Rotated,
    /// Snapshot schema differs from this binary.
    Schema,
    /// Another scan or refresh holds the lock.
    Lock,
    /// Two paths share a path hash in a diff answer.
    HashCollision,
    /// No snapshot published yet.
    NoSnapshot,
    /// The snapshot exists but this user cannot read it.
    SnapshotAccess,
    /// Operation needs root.
    NotRoot,
    /// Feature not available on this OS or host.
    NotSupported,
}

impl ErrorCode {
    pub fn exit(self) -> i32 {
        match self {
            ErrorCode::Generic
            | ErrorCode::Space
            | ErrorCode::NotRoot
            | ErrorCode::NotSupported => 1,
            ErrorCode::Usage
            | ErrorCode::Config
            | ErrorCode::GuiNoTty
            | ErrorCode::PathNotFound => 2,
            ErrorCode::NoPrev => 3,
            ErrorCode::Rotated => 4,
            ErrorCode::Schema => 5,
            ErrorCode::Lock => 6,
            ErrorCode::HashCollision => 7,
            ErrorCode::NoSnapshot => 8,
            ErrorCode::SnapshotAccess => 9,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ErrorCode::Generic => "GENERIC",
            ErrorCode::Space => "SPACE",
            ErrorCode::Usage => "USAGE",
            ErrorCode::Config => "CONFIG",
            ErrorCode::GuiNoTty => "GUI_NO_TTY",
            ErrorCode::PathNotFound => "PATH_NOT_FOUND",
            ErrorCode::NoPrev => "NO_PREV",
            ErrorCode::Rotated => "ROTATED",
            ErrorCode::Schema => "SCHEMA",
            ErrorCode::Lock => "LOCK",
            ErrorCode::HashCollision => "HASH_COLLISION",
            ErrorCode::NoSnapshot => "NO_SNAPSHOT",
            ErrorCode::SnapshotAccess => "SNAPSHOT_ACCESS",
            ErrorCode::NotRoot => "NOT_ROOT",
            ErrorCode::NotSupported => "NOT_SUPPORTED",
        }
    }

    /// HTTP status for `/api/*`.
    pub fn http_status(self) -> u16 {
        match self {
            ErrorCode::Usage | ErrorCode::Config => 400,
            ErrorCode::PathNotFound | ErrorCode::NoSnapshot | ErrorCode::NoPrev => 404,
            ErrorCode::SnapshotAccess => 403,
            ErrorCode::Rotated | ErrorCode::Lock => 409,
            ErrorCode::Schema => 409,
            _ => 500,
        }
    }

    pub fn recovery(self) -> Recovery {
        use RecoveryAction::*;
        let (action, side_effects, max_retries) = match self {
            ErrorCode::Generic => (AskHuman, "none", 0),
            ErrorCode::Space => (AskHuman, "none; the old snapshot is kept", 0),
            ErrorCode::Usage | ErrorCode::PathNotFound => (FixInput, "none", 1),
            ErrorCode::Config => (AskHuman, "none", 0),
            ErrorCode::GuiNoTty => (UseJson, "none", 0),
            ErrorCode::NoPrev => (
                Wait,
                "none; the next scheduled scan creates a previous generation",
                0,
            ),
            ErrorCode::Rotated => (Retry, "none", 3),
            ErrorCode::Schema => (Wait, "a rescan may already be running", 0),
            ErrorCode::Lock => (Wait, "none; do not start a second scan", 0),
            ErrorCode::HashCollision => (AskHuman, "none", 0),
            ErrorCode::NoSnapshot => (AskHuman, "installing starts a full scan as root", 0),
            ErrorCode::SnapshotAccess => (AskHuman, "changes snapshot directory permissions", 0),
            ErrorCode::NotRoot => (AskHuman, "none", 0),
            ErrorCode::NotSupported => (AskHuman, "none", 0),
        };
        Recovery {
            action,
            side_effects: side_effects.to_string(),
            max_retries,
        }
    }

    pub fn docs(self) -> String {
        format!(
            "https://github.com/mikhailsal/free-space#error-{}",
            self.as_str().to_lowercase().replace('_', "-")
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryAction {
    /// Retry the same command (up to `max_retries`).
    Retry,
    /// Wait for the running or next scan, then retry.
    Wait,
    /// Correct the arguments.
    FixInput,
    /// Use the `--json` commands instead.
    UseJson,
    /// Show `fix` to a human; an agent must not run it on its own.
    AskHuman,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Recovery {
    pub action: RecoveryAction,
    pub side_effects: String,
    pub max_retries: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ErrorContext {
    pub version: String,
    pub snapshot_dir: Option<String>,
    pub config: Option<String>,
    /// `state.json` last_error, when it explains this failure.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<crate::snapshot::state::LastError>,
    /// Running scan, when the lock is held.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scanning: Option<crate::snapshot::lock::Scanning>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ErrorDetail {
    pub code: ErrorCode,
    pub exit: i32,
    pub message: String,
    pub fix: Option<String>,
    pub recovery: Recovery,
    pub context: ErrorContext,
    pub docs: String,
}

/// `{"error": {...}}` printed on stdout under `--json` (D16).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ErrorBody {
    pub error: ErrorDetail,
}

#[derive(Debug, Clone)]
pub struct ErrorInner {
    pub code: ErrorCode,
    pub message: String,
    pub fix: Option<String>,
    pub last_error: Option<crate::snapshot::state::LastError>,
    pub scanning: Option<crate::snapshot::lock::Scanning>,
}

/// Boxed so `Result<T>` stays one pointer wide on the happy path.
#[derive(Debug, Clone)]
pub struct Error(Box<ErrorInner>);

impl std::ops::Deref for Error {
    type Target = ErrorInner;
    fn deref(&self) -> &ErrorInner {
        &self.0
    }
}

impl std::ops::DerefMut for Error {
    fn deref_mut(&mut self) -> &mut ErrorInner {
        &mut self.0
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({})", self.message, self.code.as_str())
    }
}

impl std::error::Error for Error {}

pub type Result<T, E = Error> = std::result::Result<T, E>;

impl Error {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Error(Box::new(ErrorInner {
            code,
            message: message.into(),
            fix: None,
            last_error: None,
            scanning: None,
        }))
    }

    pub fn with_fix(mut self, fix: impl Into<String>) -> Self {
        self.fix = Some(fix.into());
        self
    }

    pub fn usage(message: impl Into<String>) -> Self {
        Error::new(ErrorCode::Usage, message)
    }

    pub fn generic(message: impl Into<String>) -> Self {
        Error::new(ErrorCode::Generic, message)
    }

    pub fn io(context: impl std::fmt::Display, e: std::io::Error) -> Self {
        Error::new(ErrorCode::Generic, format!("{context}: {e}"))
    }

    /// Capacity errors from scan staging/spool files use the same contract as SQLite FULL.
    pub(crate) fn storage_io(context: impl std::fmt::Display, e: std::io::Error) -> Self {
        let code = match e.raw_os_error() {
            Some(libc::ENOSPC | libc::EDQUOT) => ErrorCode::Space,
            _ => ErrorCode::Generic,
        };
        Error::new(code, format!("{context}: {e}"))
    }

    pub fn exit(&self) -> i32 {
        self.code.exit()
    }

    pub fn body(&self, snapshot_dir: Option<&str>, config: Option<&str>) -> ErrorBody {
        ErrorBody {
            error: ErrorDetail {
                code: self.code,
                exit: self.exit(),
                message: self.message.clone(),
                fix: self.fix.clone(),
                recovery: self.code.recovery(),
                context: ErrorContext {
                    version: crate::VERSION.to_string(),
                    snapshot_dir: snapshot_dir.map(str::to_string),
                    config: config.map(str::to_string),
                    last_error: self.last_error.clone(),
                    scanning: self.scanning.clone(),
                },
                docs: self.code.docs(),
            },
        }
    }
}

impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Self {
        let code = if matches!(&e, rusqlite::Error::SqliteFailure(f, _)
            if f.code == rusqlite::ErrorCode::DiskFull)
        {
            ErrorCode::Space
        } else {
            ErrorCode::Generic
        };
        Error::new(code, format!("sqlite: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn staging_capacity_errors_are_space_and_other_io_errors_remain_generic() {
        for errno in [libc::ENOSPC, libc::EDQUOT] {
            let error = Error::storage_io("staging", std::io::Error::from_raw_os_error(errno));
            assert_eq!(error.code, ErrorCode::Space);
            assert_eq!(error.exit(), 1);
        }
        let error = Error::storage_io("staging", std::io::Error::from_raw_os_error(libc::EACCES));
        assert_eq!(error.code, ErrorCode::Generic);
    }

    #[test]
    fn exit_codes_match_the_contract() {
        assert_eq!(ErrorCode::Space.exit(), 1);
        assert_eq!(ErrorCode::GuiNoTty.exit(), 2);
        assert_eq!(ErrorCode::NoPrev.exit(), 3);
        assert_eq!(ErrorCode::Rotated.exit(), 4);
        assert_eq!(ErrorCode::Schema.exit(), 5);
        assert_eq!(ErrorCode::Lock.exit(), 6);
        assert_eq!(ErrorCode::HashCollision.exit(), 7);
        assert_eq!(ErrorCode::NoSnapshot.exit(), 8);
        assert_eq!(ErrorCode::SnapshotAccess.exit(), 9);
    }

    #[test]
    fn body_serializes_code_as_screaming_snake() {
        let b =
            Error::new(ErrorCode::NoSnapshot, "no snapshot").body(Some("/var/lib/freespace"), None);
        let v = serde_json::to_value(&b).unwrap();
        assert_eq!(v["error"]["code"], "NO_SNAPSHOT");
        assert_eq!(v["error"]["exit"], 8);
        assert_eq!(v["error"]["recovery"]["action"], "ask_human");
        assert!(
            v["error"]["docs"]
                .as_str()
                .unwrap()
                .ends_with("#error-no-snapshot")
        );
    }
}

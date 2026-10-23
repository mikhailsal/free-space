//! freespace core: scanner, snapshot writer and the query layer shared by the
//! CLI, the HTTP API and the agent skill.

pub mod config;
pub mod error;
pub mod format;
pub mod job;
pub mod platform;
pub mod queries;
pub mod refresh;
pub mod scan;
pub mod snapshot;

/// The one-liner every "no snapshot" message points to.
pub const INSTALL_CMD: &str =
    "curl -fsSL https://raw.githubusercontent.com/mikhailsal/free-space/master/install.sh | sh";

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub use error::{Error, ErrorCode, Result};

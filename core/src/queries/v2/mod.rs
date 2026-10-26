//! Opt-in public-query contracts over one pinned generation store.
//! Stable IDs are identities only; component keys provide traversal order.
//! Production CLI/HTTP routing continues to use the v1 entry points.

mod common;
mod diff;
mod listing;
pub mod pass;
mod status;

pub use diff::diff;
pub use listing::{find, ls, top};
pub use status::{status, status_page};

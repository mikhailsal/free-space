pub mod denied;
pub mod staged;
pub mod tree;
pub mod walk;

pub use tree::{
    Finalized, HardlinkRow, Kind, MountRow, NO_PARENT, Node, RootInfo, SubtreeAgg, Tree, flags,
};
pub use walk::{WalkOptions, default_workers, walk, walk_with_mounts};

/// Keep scanner root errors consistent across storage engines.
pub(crate) fn walk_error(error: std::io::Error) -> crate::Error {
    let code = match error.kind() {
        std::io::ErrorKind::InvalidInput | std::io::ErrorKind::NotFound => crate::ErrorCode::Usage,
        _ => crate::ErrorCode::Generic,
    };
    crate::Error::new(code, format!("scan failed: {error}"))
}

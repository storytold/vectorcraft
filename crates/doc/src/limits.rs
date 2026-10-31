use super::NodeId;

/// Coordinates beyond this many points exceed the supported document range.
pub const MAX_COORD: f64 = 4.0e6;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum DocError {
    #[error("no such object {0}")]
    NoNode(NodeId),
    #[error("object {0} cannot have children")]
    NotContainer(NodeId),
    #[error("{0}")]
    Invalid(String),
}

pub const FORMAT_VERSION: u32 = 1;

/// Node, slice and asset ids are handed out below this: no document gets near it by counting, so
/// larger ids in a file are damage (or made by hand) and the counter never follows them (#1718).
pub(crate) const MAX_NEXT_ID: u64 = 1 << 62;

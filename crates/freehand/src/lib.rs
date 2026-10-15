//! Reader for Macromedia / Aldus FreeHand drawings (`.fh*`, and FreeHand templates `.ft*`):
//! the AGD container, the record stream of FreeHand 8 through MX (11), and the subset of the
//! document model VectorCraft imports.
//!
//! The byte layout is the one described for this crate (see the README). Nothing here writes
//! FreeHand files. Product versions older than 8 are recognised and refused.
#![forbid(unsafe_code)]

mod container;
mod cursor;
mod mac;
mod model;
mod records;
#[cfg(any(test, feature = "synth"))]
pub mod synth;

pub use container::{Limits, is_freehand};
pub use model::{Document, Fill, Item, Layer, Page, Path, Rgba, Stop, Stroke, Subpath, Text, TextRun, read};

use std::fmt;

/// Why a file could not be read. The display text is the sentence shown when an open fails.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(String);

impl Error {
    pub(crate) fn new(msg: impl Into<String>) -> Self {
        Self(msg.into())
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

impl From<&'static str> for Error {
    fn from(msg: &'static str) -> Self {
        Self(msg.to_string())
    }
}

impl From<String> for Error {
    fn from(msg: String) -> Self {
        Self(msg)
    }
}

/// A count of properties, strings, dashes, stops or similar that is past the safety cap.
pub(crate) fn entries() -> Error {
    Error::new("the FreeHand file claims more entries than can be read")
}

/// A record whose on-disk size this reader does not know.
pub(crate) fn unknown_size() -> Error {
    Error::new("the FreeHand file has a record whose size is not known")
}

#[cfg(test)]
mod tests;

//! Reader for CorelDRAW drawings of versions 5 to 13 (`.cdr`, and `.cdt` templates): the RIFF
//! chunk tree with its compressed lists, then pages, layers, guidelines, groups, curves,
//! rectangles, ellipses, arcs and polygons with uniform fills and outlines.
//!
//! Every record layout the reader relies on is listed in `PROVENANCE.md` with its public source
//! and status; code cites the row ids (R1 ...). Reading is bounded by [`Limits`], never recurses and does not
//! panic. Nothing here writes CorelDRAW files.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::fmt;

mod build;
mod bytes;
mod geometry;
mod layout;
mod limits;
pub mod model;
mod records;
mod riff;
#[cfg(any(test, feature = "synth"))]
pub mod synth;

pub use limits::Limits;
pub use model::{Drawing, Warning};
pub use riff::is_cdr_riff as is_cdr;

/// Why a file can't be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// Not a RIFF CorelDRAW drawing.
    NotCdr,
    /// A drawing of a version this reader doesn't read (hundreds: 300 is version 3).
    Version(u16),
    /// A container structure that doesn't parse.
    Damaged(&'static str),
    /// Past one of the [`Limits`].
    Limit(&'static str),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::NotCdr => write!(f, "not a CorelDRAW drawing"),
            Error::Version(v) => write!(f, "version {} drawings are not read", v / 100),
            Error::Damaged(what) => write!(f, "damaged {what}"),
            Error::Limit(what) => write!(f, "past the {what} limit"),
        }
    }
}

impl std::error::Error for Error {}

/// Read a drawing within `limits`.
pub fn read(bytes: &[u8], limits: Limits) -> Result<Drawing, Error> {
    let mut budget = limits::Budget::new(limits);
    let mut warnings = Vec::new();
    let ix = riff::Index::build(bytes, &mut budget, &mut warnings)?;
    build::drawing(&ix, &mut budget, &mut warnings)
}

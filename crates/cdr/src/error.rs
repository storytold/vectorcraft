use std::fmt;

/// Why a CorelDRAW file could not be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    Malformed(&'static str),
    Unsupported(&'static str),
    Limit(&'static str),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Malformed(s) => write!(f, "damaged CorelDRAW file: {s}"),
            Self::Unsupported(s) => write!(f, "unsupported CorelDRAW file: {s}"),
            Self::Limit(s) => write!(f, "CorelDRAW file exceeds safety limit: {s}"),
        }
    }
}

impl std::error::Error for Error {}

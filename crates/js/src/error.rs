//! The error a script that cannot be read gives: a `SyntaxError` with its message and where in the
//! source it is.

use std::fmt;

/// A script that does not follow the grammar, or breaks one of its early-error rules. `line` and
/// `column` count from 1; the column counts UTF-16 code units from the start of the line, as
/// JavaScript source positions do. `offset` is the UTF-16 offset from the start of the source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyntaxError {
    pub message: String,
    pub line: u32,
    pub column: u32,
    pub offset: u32,
}

impl fmt::Display for SyntaxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SyntaxError: {} (line {}, column {})", self.message, self.line, self.column)
    }
}

impl std::error::Error for SyntaxError {}

/// The message of the error nesting deeper than [`crate::Options::max_depth`] gives.
pub const TOO_DEEP: &str = "nesting too deep";

/// The message of the error a source longer than [`crate::Options::max_source_units`] gives.
pub const TOO_LONG: &str = "source too long";

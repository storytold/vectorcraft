//! Integration tests: one test binary for the crate, one module per area.
// Unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod cli;
mod fileio;
mod mcp_e2e;

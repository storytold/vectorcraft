//! Integration tests: one test binary for the crate, one module per area.
// Unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod cli;
mod fileio;
mod mcp_e2e;
mod script;

/// Nextest relocates archived binaries; Cargo's compile-time path still names the build runner.
fn bin() -> std::path::PathBuf {
    std::env::var_os("NEXTEST_BIN_EXE_vectorcraft_cli")
        .or_else(|| std::env::var_os("CARGO_BIN_EXE_vectorcraft-cli"))
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_vectorcraft-cli").into())
}

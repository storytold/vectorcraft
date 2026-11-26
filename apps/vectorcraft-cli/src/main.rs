//! The `vectorcraft-cli` binary: wires the process to [`vectorcraft_cli::run`].
#![forbid(unsafe_code)]

use std::process::ExitCode;

fn main() -> ExitCode {
    vectorcraft_cli::init();
    // The handles, not locks: the MCP server writes to stdout from threads of its own.
    let code = vectorcraft_cli::run(std::env::args_os().skip(1), &mut std::io::stdout(), &mut std::io::stderr());
    ExitCode::from(u8::try_from(code).unwrap_or(1))
}

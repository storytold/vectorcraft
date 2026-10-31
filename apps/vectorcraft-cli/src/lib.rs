//! `vectorcraft-cli`: VectorCraft from the command line (RFC-0012). `run` is the whole program, so
//! it can be tested in-process as well as through the binary.
//!
//! ```text
//! vectorcraft-cli mcp [--connect ADDR | --headless] [--automation-read-root DIR]... [--automation-write-root DIR]...
//! vectorcraft-cli run [--in FILE] [--cmd ID [--params JSON] | --commands FILE|-]... [--export FILE]... [--scale N]
//! vectorcraft-cli commands
//! vectorcraft-cli convert IN OUT [--scale N] [--artboard I | --range R] [--outline-text] [--decimals N] [--replace-lossy] [--editing-data] [--preserve-editing] [--ai-version 7|pdf] [--hidden-objects layer|omit|visible] [--transparent-images flatten|omit]
//! vectorcraft-cli info FILE
//! vectorcraft-cli bench FILE [--size WxH] [--iters N]
//! vectorcraft-cli perf [--paths N] [--deep]
//! ```
// Denied, not forbidden: the font listers shared with the desktop app allow it for their
// DirectWrite (Windows) and CoreText (macOS) calls, and nothing else may.
#![deny(unsafe_code)]

use std::ffi::OsString;
use std::io::Write;

mod batch;
mod cli;
mod convert;
mod error;
mod inspect;
/// The desktop app's CoreText font lister, shared: exports and MCP find the same fonts.
#[cfg(target_os = "macos")]
#[path = "../../vectorcraft/src/mac_fonts.rs"]
mod mac_fonts;
mod mcp;
mod perf;
/// The desktop app's settings folder, shared: exports and MCP read the same Fonts folder.
#[path = "../../vectorcraft/src/prefs_dir.rs"]
mod prefs_dir;
/// The desktop app's DirectWrite font lister, shared: exports and MCP find the same fonts.
#[cfg(windows)]
#[path = "../../vectorcraft/src/system_fonts.rs"]
mod system_fonts;

pub use cli::command;
use cli::{Cli, Command};
use error::CliError;

/// Registers the fonts that font services load and the Fonts folder of the preferences (#579). Call
/// it once, before any font lookup.
pub fn init() {
    #[cfg(windows)]
    system_fonts::install();
    #[cfg(target_os = "macos")]
    mac_fonts::install();
    prefs_dir::install_fonts();
}

/// Runs the CLI on its arguments (without the program name). Returns the process exit code: 0 for
/// success, help and `--version`, 1 for a failure, 2 for a usage error. clap reads the arguments
/// as they are, so one that is not valid Unicode is a usage error, not a panic. A reader that
/// stopped early (`vectorcraft-cli commands | head`) ends the program quietly with 0, as ripgrep
/// does.
pub fn run<I, T>(args: I, out: &mut dyn Write, err: &mut dyn Write) -> i32
where
    I: IntoIterator<Item = T>,
    T: Into<OsString>,
{
    let args: Vec<OsString> = args.into_iter().map(Into::into).collect();
    // Help and `--version` come back as errors too, for `out` with exit 0: nothing runs.
    let Cli { command } = match cli::parse(&args) {
        Ok(cli) => cli,
        Err(e) => {
            let to: &mut dyn Write = if e.use_stderr() { err } else { out };
            // Nothing more can be done about a stream that can't be written.
            let _ = write!(to, "{}", e.render());
            return e.exit_code();
        }
    };
    let result = match &command {
        Command::Mcp(a) => mcp::mcp(a, err),
        Command::Run(a) => batch::run(a, out),
        Command::Commands => inspect::commands(out),
        Command::Convert(a) => convert::convert(a, out),
        Command::Info(a) => inspect::info(a, out),
        Command::Bench(a) => inspect::bench(a, out),
        Command::Perf(a) => perf::run(a, out).map_err(CliError::from),
    };
    let result = result.and_then(|()| out.flush().map_err(CliError::Stdout));
    match result {
        Ok(()) => 0,
        Err(e) if e.is_closed_stdout() => 0,
        Err(e) => {
            // As above: if stderr is gone too, the exit code is all that is left.
            let _ = writeln!(err, "error: {e}");
            1
        }
    }
}

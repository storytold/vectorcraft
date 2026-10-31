//! Why a subcommand failed. Printed as `error: {e}`, exit 1; usage errors (exit 2) are clap's.
//!
//! The engine and MCP layers report failures as text; they are carried as the `source` of the
//! variant that says what the CLI was doing.

/// Why a subcommand failed.
#[derive(Debug, thiserror::Error)]
pub(crate) enum CliError {
    /// Writing a result to standard output failed (a closed pipe is not an error: see [`crate::run`]).
    #[error("can't write to stdout: {0}")]
    Stdout(#[source] std::io::Error),
    #[error("{0}")]
    Engine(String),
    #[error("open {path}: {message}")]
    Open { path: String, message: String },
    #[error("export {path}: {message}")]
    Export { path: String, message: String },
    #[error("step {step} ({id}): {message}")]
    Step { step: usize, id: String, message: String },
    #[error("step {step}: export {path}: {message}")]
    StepExport { step: usize, path: String, message: String },
    #[error(transparent)]
    Commands(#[from] CommandsError),
    #[error("{0}")]
    Roots(String),
    #[error("cannot connect to {addr}: {source}")]
    Connect { addr: String, source: std::io::Error },
    #[error("MCP server: {0}")]
    Serve(#[source] std::io::Error),
    #[error("document has no artboard")]
    NoArtboard,
    #[error(transparent)]
    Perf(#[from] crate::perf::PerfError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

/// Why the steps of `run --commands SOURCE` couldn't be read; `source` is the file, or `-`.
#[derive(Debug, thiserror::Error)]
pub(crate) enum CommandsError {
    #[error("--commands {file}: {error}")]
    Read { file: String, error: std::io::Error },
    #[error("--commands {file}: more than {mib} MiB of steps")]
    TooLarge { file: String, mib: u64 },
    #[error("--commands {file}: not UTF-8 text")]
    NotUtf8 { file: String },
    #[error("--commands {file}: more than {max} steps")]
    TooManySteps { file: String, max: usize },
    #[error("--commands {file}: {error}")]
    Json { file: String, error: serde_json::Error },
    #[error("--commands {file}: not a JSON array")]
    NotArray { file: String },
    #[error("--commands {file}: step {step} (line {line}): {error}")]
    Line { file: String, step: usize, line: usize, error: serde_json::Error },
    #[error("--commands {file}: step {step} ({place}): {error}")]
    Step { file: String, step: usize, place: String, error: StepError },
}

/// What is wrong with one entry of a `--commands` file.
#[derive(Debug, thiserror::Error)]
pub(crate) enum StepError {
    #[error("a step must be an object such as {{\"command\": \"select.all\", \"params\": {{}}}}")]
    NotObject,
    #[error("give `command` or `id`, not both")]
    CommandAndId,
    #[error("no `command`")]
    NoCommand,
    #[error("`command` must be a string, a command id")]
    CommandNotString,
    #[error("`params` of {0} must be a JSON object")]
    ParamsNotObject(String),
    #[error("unknown key `{0}`: a step has `command` (or `id`) and `params`")]
    UnknownKey(String),
}

impl CliError {
    /// Did writing a result fail because its reader had gone (a closed pipe)? That ends the
    /// program quietly, not as a failure.
    pub(crate) fn is_closed_stdout(&self) -> bool {
        match self {
            Self::Stdout(e) | Self::Perf(crate::perf::PerfError::Stdout(e)) => e.kind() == std::io::ErrorKind::BrokenPipe,
            _ => false,
        }
    }
}

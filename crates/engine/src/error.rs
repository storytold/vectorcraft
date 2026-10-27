use super::*;

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("unknown command `{0}`")]
    UnknownCommand(String),
    /// An unknown command id, with the command id it most likely misspells.
    #[error("unknown command `{0}`; did you mean `{1}`?")]
    UnknownCommandNear(String, String),
    #[error("command `{0}` is not available right now: {1}")]
    Disabled(String, String),
    #[error("invalid parameters for `{cmd}`: {msg}")]
    BadParams { cmd: String, msg: String },
    #[error("no active document")]
    NoDocument,
    #[error("no such object {0}")]
    NoNode(NodeId),
    #[error("{0}")]
    Other(String),
    /// The caller asked for the command to stop ([`cancel`]). It changed nothing.
    #[error("cancelled; nothing was changed")]
    Cancelled,
    /// A bug: the command panicked. The document was rolled back to its state before the command.
    #[error("internal error in `{cmd}`: {msg} (the document was left as it was before; please report this bug)")]
    Internal { cmd: String, msg: String },
}

impl From<cancel::Cancelled> for EngineError {
    fn from(_: cancel::Cancelled) -> Self {
        EngineError::Cancelled
    }
}

impl From<vectorcraft_doc::DocError> for EngineError {
    fn from(e: vectorcraft_doc::DocError) -> Self {
        EngineError::Other(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, EngineError>;

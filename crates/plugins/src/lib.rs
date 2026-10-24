//! Sandboxed WebAssembly plug-ins for VectorCraft.
//!
//! A plug-in is a WebAssembly module that implements the VectorCraft plug-in ABI (v1, documented
//! in `docs/plugins.md`). It runs in [`wasmi`], a pure-Rust interpreter, with:
//!
//! - **no host imports**: no WASI, file system, network, clock or randomness. A module that
//!   imports anything is rejected;
//! - an **instruction budget** (fuel) per call, metered in slices so a wall-clock deadline is
//!   also enforced (native builds), so infinite loops end with an error. A filter may ask for a
//!   larger one ([`Budget::Heavy`]), which the host lets the user allow before installing it;
//! - a **linear-memory cap**, a **recursion-depth cap** and wasmi's strict module limits;
//! - a **fresh instance per run**, so no state leaks between calls.
//!
//! Every failure (malformed module, trap, exhausted budget, bad output) is an [`Error`]; nothing
//! a module does can panic or hang the host.
//!
//! Plug-ins see the document as JSON ([`objects`]): the selected paths and compound paths with
//! their geometry, fills, strokes (gradients included) and opacity. An **object filter** returns
//! the objects to keep, change or add, applied to the document as one edit
//! ([`objects::apply_output`]); a **live effect** ([`effect`]) rewrites one object's geometry each
//! time it is drawn, from an appearance stack, and may show lines on the canvas beside it
//! ([`overlay`]); an **exporter** ([`export`]) writes the whole
//! document in a file format of its own. The process-wide [`registry`] holds the installed
//! plug-ins. A plug-in's parameters ([`manifest`]) come with what the host's dialogs need to show
//! them well: labels, units, groups, help and conditions ([`dialog`]).
#![forbid(unsafe_code)]

pub mod dialog;
pub mod effect;
pub mod export;
pub mod manifest;
pub mod objects;
pub mod overlay;
pub mod registry;
mod runtime;
pub mod wat;

pub use dialog::ParamUi;
pub use manifest::{Budget, Kind, Manifest, Param, ParamSpec};
pub use runtime::{ABI_VERSION, HEAVY_FUEL, HEAVY_WALL_TIME_MS, Limits, Plugin, heavy_terms};

/// What the host puts before a plug-in's id to name what it adds: its live effect
/// (`plugin.<id>`, [`effect`]) or its file format ([`export`]).
pub const PREFIX: &str = "plugin.";

/// A plug-in error. Every way a module can misbehave ends here, never in a panic.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("not a valid plug-in module: {0}")]
    Module(String),
    #[error("plug-in ABI: {0}")]
    Abi(String),
    #[error("plug-in manifest: {0}")]
    Manifest(String),
    #[error("plug-in trapped: {0}")]
    Trap(String),
    #[error("plug-in exceeded its {0}")]
    Limit(String),
    #[error("plug-in parameters: {0}")]
    Params(String),
    #[error("plug-in output: {0}")]
    Output(String),
    #[error("plug-in failed: {0}")]
    Failed(String),
    #[error("no plug-in {0:?} is installed")]
    NotFound(String),
    /// The caller asked the run to stop ([`vectorcraft_doc::cancel`]), between two slices of
    /// its instruction budget.
    #[error("cancelled")]
    Cancelled,
    #[error("{0}")]
    Io(String),
}

pub type Result<T> = std::result::Result<T, Error>;

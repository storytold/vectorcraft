//! VectorCraft's MCP server.
//!
//! [Model Context Protocol](https://modelcontextprotocol.io) over stdio: newline-delimited
//! JSON-RPC 2.0, hand-written (no async runtime). The server exposes VectorCraft as a set of MCP
//! tools and resources and forwards everything to a [`Backend`]:
//!
//! - [`Remote`] talks to a running desktop app through its loopback JSON-lines control channel
//!   (`vectorcraft --control 7979`): one `{"id","method","params"}` line in, one
//!   `{"id","ok","result"|"error"}` line out.
//! - [`Headless`] hosts an in-process [`vectorcraft_engine::Session`] and implements the same
//!   control-channel method names itself (rendering screenshots with `vectorcraft-render`), so agents
//!   can draw and look at the result without a window.
//!
//! Entry points: [`Server::serve`] (stdio loop) and [`Server::handle_line`] (one message).
#![forbid(unsafe_code)]

mod backend;
mod headless;
pub mod logging;
mod prompts;
mod resources;
pub mod roots;
mod server;
mod tools;

pub use backend::{Backend, Remote};
pub use headless::Headless;
pub use prompts::{PROMPTS, PromptArg, PromptDef};
pub use resources::{DOC_JSON_URI, DOC_URI, TEMPLATES};
pub use roots::FileRoots;
pub use server::{PROTOCOL_VERSION, Server};
pub use tools::{ToolResult, call_tool, call_tool_confined, tool_definitions};

/// Default control-channel address of the desktop app.
pub const DEFAULT_ADDR: &str = "127.0.0.1:7979";

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_conventions;
#[cfg(test)]
mod tests_distortkeys;
#[cfg(test)]
mod tests_exportas;
#[cfg(test)]
mod tests_fileio;
#[cfg(test)]
mod tests_freeform;
#[cfg(test)]
mod tests_gradient;
#[cfg(test)]
mod tests_links;
#[cfg(test)]
mod tests_liquify;
#[cfg(test)]
mod tests_pen;
#[cfg(test)]
mod tests_persp;
#[cfg(test)]
mod tests_place;
#[cfg(test)]
mod tests_protocol;
#[cfg(test)]
mod tests_roots;
#[cfg(test)]
mod tests_svg;

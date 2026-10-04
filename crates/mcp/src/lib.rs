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
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod backend;
mod headless;
mod server;
mod tools;

pub use backend::{Backend, Remote};
pub use headless::Headless;
pub use server::{PROTOCOL_VERSION, Server};
pub use tools::{ToolResult, call_tool, tool_definitions};

/// Default control-channel address of the desktop app.
pub const DEFAULT_ADDR: &str = "127.0.0.1:7979";

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_fileio;
#[cfg(test)]
mod tests_gradient;

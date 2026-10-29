//! MCP tool definitions (names, descriptions, JSON Schemas) and their implementations on top of a
//! [`Backend`].

use serde_json::{Map, Value, json};
use vectorcraft_engine::cmd::fileio::{ARTBOARD_PARAMS, FORMATS, OPEN_EXTS, SAVE_FORMATS, format, save_reads_option, writable_format};

use crate::backend::Backend;

mod definitions;
mod dispatch;
mod handlers;

pub use definitions::tool_definitions;
#[cfg(test)]
pub(crate) use dispatch::MAX_COMMAND_BATCH_STEPS;
pub use dispatch::call_tool;
pub(crate) use dispatch::check_tool_arguments;

/// The result of `tools/call`: MCP content blocks plus the `isError` flag.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolResult {
    pub content: Vec<Value>,
    pub is_error: bool,
}

impl ToolResult {
    pub fn text(t: impl Into<String>) -> Self {
        Self { content: vec![json!({"type": "text", "text": t.into()})], is_error: false }
    }
    pub fn json(v: &Value) -> Self {
        Self::text(serde_json::to_string_pretty(v).unwrap_or_default())
    }
    pub fn error(t: impl Into<String>) -> Self {
        Self { content: vec![json!({"type": "text", "text": t.into()})], is_error: true }
    }
    pub fn to_value(&self) -> Value {
        json!({"content": self.content, "isError": self.is_error})
    }
}

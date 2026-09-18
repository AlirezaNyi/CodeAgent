//! Tool call / result models.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::ids::ToolCallId;

/// A request to invoke a named tool.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolCall {
    pub id: ToolCallId,
    pub name: String,
    pub input: Value,
}

impl ToolCall {
    pub fn new(name: impl Into<String>, input: Value) -> Self {
        Self {
            id: ToolCallId::new(),
            name: name.into(),
            input,
        }
    }
}

/// Result of a tool execution.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolResult {
    pub call_id: ToolCallId,
    pub name: String,
    pub success: bool,
    pub output: String,
    pub truncated: bool,
    pub error: Option<String>,
    pub duration_ms: u64,
}

impl ToolResult {
    pub fn ok(call_id: ToolCallId, name: impl Into<String>, output: impl Into<String>) -> Self {
        Self {
            call_id,
            name: name.into(),
            success: true,
            output: output.into(),
            truncated: false,
            error: None,
            duration_ms: 0,
        }
    }

    pub fn err(call_id: ToolCallId, name: impl Into<String>, error: impl Into<String>) -> Self {
        Self {
            call_id,
            name: name.into(),
            success: false,
            output: String::new(),
            truncated: false,
            error: Some(error.into()),
            duration_ms: 0,
        }
    }
}

/// JSON Schema description of a tool's input (external boundary).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

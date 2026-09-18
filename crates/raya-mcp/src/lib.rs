//! MCP stdio control plane for RAYA Agent.
//!
//! Exposes task run/status/approve/memory/dag tools for Cursor. Does **not**
//! expose filesystem or shell tools — those remain inside the orchestrator
//! behind [`raya_policy::PolicyEngine`].

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod ops;
mod server;

pub use ops::{McpOpError, RayaMcpContext, tool_names};
pub use server::{RayaMcpServer, serve_stdio};

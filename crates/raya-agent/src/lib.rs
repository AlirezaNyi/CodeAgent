//! Agent orchestrator and resource management.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod orchestrator;
mod resources;
mod subagent;

pub use orchestrator::{Orchestrator, OrchestratorError};
pub use resources::{ResourceLimits, ResourceManager, ResourcePermit};
pub use subagent::{SubagentBrief, SubagentOutcome, SubagentRole, SubagentRunner, SubagentVerdict};

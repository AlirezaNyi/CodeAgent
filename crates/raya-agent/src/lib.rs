//! Agent orchestrator and resource management.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod orchestrator;
mod resources;

pub use orchestrator::{Orchestrator, OrchestratorError};
pub use resources::{ResourceLimits, ResourceManager, ResourcePermit};

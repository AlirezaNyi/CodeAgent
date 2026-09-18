//! Bounded ephemeral subagents (Phase 3 slice A).

mod brief;
mod role;
mod runner;

pub use brief::{SubagentBrief, SubagentOutcome, SubagentVerdict};
pub use role::SubagentRole;
pub use runner::SubagentRunner;

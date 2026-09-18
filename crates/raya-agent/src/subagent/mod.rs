//! Bounded ephemeral subagents (Phase 3 slice A) + optional DAG (Slice C).

mod brief;
mod dag;
mod role;
mod runner;

pub use brief::{SubagentBrief, SubagentOutcome, SubagentVerdict};
pub use dag::{all_terminal, ready_nodes, skip_blocked};
pub use role::SubagentRole;
pub use runner::SubagentRunner;

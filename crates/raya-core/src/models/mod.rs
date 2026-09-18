//! Typed domain models for tasks, plans, tools, LLM I/O, events, and policy.

mod decision;
mod event;
mod ids;
mod llm;
mod plan;
mod policy;
mod task;
mod tool;

pub use decision::AgentDecision;
pub use event::{Event, EventKind};
pub use ids::{EventId, ProjectId, TaskId, ToolCallId};
pub use llm::{CompletionRequest, CompletionResponse, Message, MessageRole, TokenUsage};
pub use plan::{ExecutionPlan, PlanStep, VerificationStrategy};
pub use policy::{PolicyDecision, RiskClass};
pub use task::{AgentTask, TaskPhase, TaskStatus};
pub use tool::{ToolCall, ToolResult, ToolSchema};

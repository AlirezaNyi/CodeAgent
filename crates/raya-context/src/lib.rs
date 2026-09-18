//! Bounded context retrieval: search → rank → token budget.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod engine;
mod rank;
mod search;

pub use engine::{ContextBundle, ContextEngine, ContextError, ContextSnippet};
pub use rank::{RankExplanation, RankSignals, RankedCandidate};

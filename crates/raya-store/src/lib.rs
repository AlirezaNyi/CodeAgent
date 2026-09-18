//! SQLite persistence for RAYA Agent.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod error;
mod store;

pub use error::{StoreError, StoreResult};
pub use store::{ProjectRecord, Store};

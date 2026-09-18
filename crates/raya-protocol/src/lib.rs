//! Localhost HTTP API.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod server;

pub use server::{AppState, bind_addr, router, serve};

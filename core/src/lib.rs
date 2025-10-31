//! # razor-rpc-core
//!
//! This crate provides the core utilities for [`razor-rpc`](https://docs.rs/razor-rpc).
//! It includes common types, traits, and error handling mechanisms used by other crates in the workspace.

mod codec;
pub use codec::Codec;
mod config;
pub use config::*;
pub mod buffer;
pub mod error;

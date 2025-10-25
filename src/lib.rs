#![cfg_attr(docsrs, feature(doc_cfg))]
#![cfg_attr(docsrs, allow(unused_attributes))]

//! # occams-rpc
//!
//! This crate provides a high-level remote API call interface for `occams-rpc`.
//! It is part of a modular, pluggable RPC for high throughput scenarios that supports various async runtimes.
//!
//! Currently a placeholder, API should be added in the future
//!
//! If you are looking for streaming interface, use [occams-rpc-stream](https://docs.rs/occams-rpc-stream) instead.
//!
//!
//! ## Components
//!
//! `occams-rpc` is built from a collection of crates that provide different functionalities:
//!
//! - [`occams-rpc-core`](https://docs.rs/occams-rpc-core): core utils crate
//! - [`occams-rpc-codec`](https://docs.rs/occams-rpc-codec): Provides codecs for serialization, such as `msgpack`.
//! - runtimes:
//!   - [`occams-rpc-tokio`](https://docs.rs/occams-rpc-tokio): A runtime adapter for the `tokio` runtime.
//!   - [`occams-rpc-smol`](https://docs.rs/occams-rpc-smol): A runtime adapter for the `smol` runtime.
//! - transports:
//!   - [`occams-rpc-tcp`](https://docs.rs/occams-rpc-tcp): A TCP transport implementation.

#[cfg(feature = "tokio")]
pub type RT = occams_rpc_tokio::TokioRT;
#[cfg(all(not(feature = "tokio"), feature = "smol"))]
pub type RT = occams_rpc_smol::SmolRT;

pub mod client;
pub mod server;

// re-export for macros, so that user don't need to use multiple crates
pub use occams_rpc_core::{
    Codec,
    error::{RpcErrCodec, RpcError, RpcIntErr},
};

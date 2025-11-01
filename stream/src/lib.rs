#![cfg_attr(docsrs, feature(doc_cfg))]
#![cfg_attr(docsrs, allow(unused_attributes))]

//! # razor-stream
//!
//! This crate provides a low-level streaming interface for `razor-rpc`.
//! It is used for stream processing and is part of the modular design of `razor-rpc`.
//!
//! If you are looking for a high-level remote API call interface, use [`razor-rpc`](https://docs.rs/razor-rpc) instead.
//!
//! ## Components
//!
//! `razor-rpc` is designed to be modular and pluggable. It is a collection of crates that provide different functionalities:
//!
//! - [`razor-rpc-codec`](https://docs.rs/razor-rpc-codec): Provides codecs for serialization, such as `msgpack`.
//! - Async runtime support by [`orb`](https://docs.rs/orb):
//!   - [`orb-tokio`](https://docs.rs/orb-tokio): A runtime adapter for the `tokio` runtime.
//!   - [`orb-smol`](https://docs.rs/orb-smol): A runtime adapter for the `smol` runtime.
//! - Transports can be implemented with a raw socket, without the overhead of the HTTP protocol:
//!   - [`razor-rpc-tcp`](https://docs.rs/razor-rpc-tcp): A TCP transport implementation.
//!
//! ## The Design
//!
//! Our implementation is designed to optimize throughput and lower
//! CPU consumption for high-performance services.
//!
//! Each connection is a full-duplex, multiplexed stream.
//! There's a `seq` ID assigned to a packet to track
//! a request and response. The timeout of a packet is checked in batches every second.
//! We utilize the [crossfire](https://docs.rs/crossfire) channel for parallelizing the work with
//! coroutines.
//!
//! With an [ClientStream](crate::client::stream::ClientStream), the request packets sent in sequence,
//! and wait with a sliding window throttler controlling the number of in-flight packets.
//! An internal timer then registers the request through a channel, and when the response
//! is received, it can optionally notify the user through a user-defined channel or another mechanism.
//!
//! [ClientPool](crate::client::ClientPool) and [FailoverPool](crate::client::FailoverPool) are provided on top of `ClientStream` for user.
//!
//! In an [RpcServer](crate::server::RpcServer), for each connection, there is one coroutine to read requests and one
//! coroutine to write responses. Requests can be dispatched with a user-defined
//! [Dispatch](crate::server::dispatch::Dispatch) trait implementation.
//!
//! Responses are received through a channel wrapped in [RespNoti](crate::server::task::RespNoti).
//!
//! ## Protocol
//!
//! The details are described in [crate::proto].
//!
//! The packet starts with a fixed-length header and is followed by a variable-length body.
//! An [RpcAction](crate::proto::RpcAction) represents the type of packet.
//! The action type is either numeric or a string.
//!
//! The request body contains a mandatory structured message and optional blob data.
//!
//! The response for each request either returns successfully with an optional structured message and
//! optional blob data (the response can be empty), or it returns with an RpcError. The error type can
//! be numeric (like a Unix errno), text (for user-customized errors), or a statically predefined error
//! string (for errors that occur during socket communication or encoding/decoding)
//!
//! ## Usage
//!
//! You can refer to the [test case](https://github.com/NaturalIO/razor-rpc/blob/master/test-suite/src/stream/) for example.
//!

#[macro_use]
extern crate captains_log;

pub mod buffer;
pub mod client;
pub mod error;
pub mod proto;
pub mod server;
// re-export for macros, so that user don't need to use multiple crates
pub use razor_rpc_codec::Codec;

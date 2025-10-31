#![cfg_attr(docsrs, feature(doc_cfg))]
#![cfg_attr(docsrs, allow(unused_attributes))]

//! # razor-rpc-codec
//!
//! This crate provides [razor_rpc_core::Codec](https://docs.rs/razor-rpc-core/latest/razor_rpc_core/trait.Codec.html) implementations for [`razor-rpc`](https://docs.rs/razor-rpc) and [`razor-rpc-stream`](https://docs.rs/razor-rpc-stream).
//! It supports different serialization formats, such as `msgpack`.

pub use razor_rpc_core::Codec;
#[cfg(feature = "msgpack")]
mod msgpack;
#[cfg(feature = "msgpack")]
pub use msgpack::*;

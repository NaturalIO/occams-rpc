#![cfg_attr(docsrs, feature(doc_cfg))]
#![cfg_attr(docsrs, allow(unused_attributes))]

//! # razor-rpc-codec
//!
//! This crate provides some core trait and codec implementations for [`razor-rpc`](https://docs.rs/razor-rpc) and [`razor-stream`](https://docs.rs/razor-stream).
//!

/*
 *  Note that there's no unify output interface in each serde impl,
 *  whatever we want to serialize into (std::io::Write / Buffer/ Vec<u8>),
 *  require the codec implement to match.
 */

use serde::{Deserialize, Serialize};

/// The codec is immutable, if need changing (like setting up cipher), should have inner
/// mutablilty
pub trait Codec: Default + Send + Sync + Sized + 'static {
    fn encode<T: Serialize>(&self, task: &T) -> Result<Vec<u8>, ()>;

    /// sererialized the msg into buf (with std::io::Writer), and return the size written
    fn encode_into<T: Serialize>(&self, task: &T, buf: &mut Vec<u8>) -> Result<usize, ()>;

    fn decode<'a, T: Deserialize<'a>>(&self, buf: &'a [u8]) -> Result<T, ()>;
}

#[cfg(feature = "msgpack")]
mod msgpack;
#[cfg(feature = "msgpack")]
pub use msgpack::*;

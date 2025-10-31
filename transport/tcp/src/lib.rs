#![cfg_attr(docsrs, feature(doc_cfg))]
#![cfg_attr(docsrs, allow(unused_attributes))]

//! # razor-rpc-tcp
//!
//! This crate provides a TCP transport implementation for [`razor-stream`](https://docs.rs/razor-stream).
//! It is used for both client and server communication over TCP.

#[macro_use]
extern crate captains_log;
mod client;
pub use client::*;
mod server;
pub use server::*;

#[macro_export(local_inner_macros)]
macro_rules! io_with_timeout {
    ($IO: path, $timeout: expr, $f: expr) => {{
        if $timeout == Duration::from_secs(0) {
            $f.await
        } else {
            // the crate reference make this macro not exportable
            match <$IO as orb::time::AsyncTime>::timeout($timeout, $f).await {
                Ok(Ok(r)) => Ok(r),
                Ok(Err(e)) => Err(e),
                Err(_) => Err(std::io::ErrorKind::TimedOut.into()),
            }
        }
    }};
}

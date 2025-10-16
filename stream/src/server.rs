//! This module contains traits defined for the server-side
//!

use std::{fmt, future::Future, io, sync::Arc};

use crate::proto::RpcAction;
use captains_log::filter::Filter;
use crossfire::*;
use io_buffer::Buffer;
pub use occams_rpc_core::ServerConfig;
use occams_rpc_core::{Codec, error::*, io::*, runtime::AsyncIO};

/// A central hub defined by the user for the server-side, to define the customizable plugin.
pub trait ServerFactory: Sync + Send + 'static + Sized {
    /// A [captains-log::filter::Filter](https://docs.rs/captains-log/latest/captains_log/filter/index.html) implementation.
    /// The type that new_logger returns.
    ///
    /// maybe a `Arc<LogFilter> or KeyFilter<Arc<LogFilter>>`
    type Logger: Filter + Send + 'static;

    /// Define the transport layer protocol
    ///
    /// Refers to [ServerTransport]
    type Transport: ServerTransport<Self>;

    /// Define the adaptor of async runtime
    ///
    /// Refers to [occams_rpc_core::runtime::AsyncIO](https://docs.rs/occams-rpc-core/latest/occams_rpc_core/runtime/index.html)
    type IO: AsyncIO;

    /// You should keep ServerConfig inside ServerFactory, get_config() will return the reference.
    fn get_config(&self) -> &ServerConfig;

    /// Construct a [captains_log::filter::Filter](https://docs.rs/captains-log/latest/captains_log/filter/trait.Filter.html) to oganize log of a client
    ///
    /// maybe a `Arc<LogFilter>` or `KeyFilter<Arc<LogFilter>>`
    fn new_logger(&self) -> Self::Logger;

    /// Define how the async runtime spawn a task
    ///
    /// You may spawn with globally runtime, or to a owned runtime executor
    fn spawn_detach<F, R>(&self, f: F)
    where
        F: Future<Output = R> + Send + 'static,
        R: Send + 'static;

    /// One of RespReceiver impl:
    /// - [crate::server_impl::RespReceiverTask]
    /// - [crate::server_impl::RespReceiverBuf]
    type RespReceiver: RespReceiver;

    /// Called when a server stream is established, initialize a ReqDispatch for the connection.
    ///
    /// The dispatch is likely to be a closure or object, in order to dispatch tasks to different workers
    fn new_dispatcher(&self) -> impl ReqDispatch<Self::RespReceiver>;
}

/// This trait is for server-side transport layer protocol.
///
/// The implementation can be found on:
///
/// - [occams-rpc-tcp](https://docs.rs/occams-rpc-tcp): For TCP and Unix socket
pub trait ServerTransport<F: ServerFactory>: Send + Sync + Sized + 'static + fmt::Debug {
    type Listener: AsyncListener;

    /// The ServerTransport holds a logger, the server will use it by reference.
    fn get_logger(&self) -> &F::Logger;

    /// The implementation is expected to store the conn_count until dropped
    fn new_conn(
        stream: <Self::Listener as AsyncListener>::Conn, f: &F, conn_count: Arc<()>,
    ) -> Self;

    /// Read a request from the socket
    fn read_req<'a>(
        &'a self, close_ch: &crossfire::MAsyncRx<()>,
    ) -> impl Future<Output = Result<RpcSvrReq<'a>, RpcIntErr>> + Send;

    /// Write out the encoded request task
    fn write_resp(
        &self, seq: u64, res: Result<(Vec<u8>, Option<Buffer>), EncodedErr>,
    ) -> impl Future<Output = io::Result<()>> + Send;

    /// Flush the response for the socket writer, if the transport has buffering logic
    fn flush_resp(&self) -> impl Future<Output = io::Result<()>> + Send;

    /// Shutdown the write direction of the connection
    fn close_conn(&self) -> impl Future<Output = ()> + Send;
}

/// A temporary struct to hold data buffer return by ServerTransport
///
/// NOTE: `RpcAction` and `msg` contains slice that reference to ServerTransport's internal buffer,
/// you should parse and clone them.
#[derive(Debug)]
pub struct RpcSvrReq<'a> {
    pub seq: u64,
    pub action: RpcAction<'a>,
    pub msg: &'a [u8],
    pub blob: Option<Buffer>, // for write, this contains data
}

/// A temporary struct to hold pre encoded buffer for RespReceiverBuf
#[derive(Debug)]
pub struct RpcSvrResp {
    pub seq: u64,

    pub msg: Option<Vec<u8>>,

    pub blob: Option<Buffer>,

    pub res: Result<(), EncodedErr>,
}

/// ReqDispatch should be a user-defined struct initialized for every connection, by ServerFactory::new_dispatcher.
///
/// ReqDispatch must have Sync, because the connection reader coroutine calls dispatch_req(),
/// and encode_resp() called by connection writer coroutine.
/// It should incorporate a RespReceiver trait instance for the writer side.
///
/// A `Codec` should be created and holds inside, shared by the read/write coroutine.
/// If you have encryption in the Codec, it could have shared states.
pub trait ReqDispatch<R: RespReceiver>: Send + Sync + Sized + 'static {
    /// Decode and handle the request, called from the connection reader coroutine.
    ///
    /// You can dispatch them to a worker pool.
    /// If you are processing them directly in the connection coroutine, should make sure not
    /// blocking the thread for long.
    /// This is an async fn, but you should avoid waiting as much as possible.
    /// Should return Err(()) when codec decode_req failed.
    fn dispatch_req<'a>(
        &'a self, req: RpcSvrReq<'a>, noti: RespNoti<R::ChannelItem>,
    ) -> impl Future<Output = Result<(), ()>> + Send;

    /// A delegate function to RespReceiver trait with owned codec
    /// Called from the response writer
    fn encode_resp(
        &self, task: R::ChannelItem,
    ) -> (u64, Result<(Vec<u8>, Option<Buffer>), EncodedErr>);
}

/// Trait to be incorporated in ReqDispatch trait, which defined the response channel item type.
///
/// There are two types of implement:
/// - [crate::server_impl::RespReceiverTask]: When you have all types of server response tasks in one enum type.
/// - [crate::server_impl::RespReceiverBuf]: When you have different types of task handled by different worker pools.
/// Task can be encoded to RpcSvrResp before sent into channel.
pub trait RespReceiver: Send + 'static {
    type ChannelItem: Send + Unpin + 'static + fmt::Debug;

    /// NOTE: Because the msg encoded from task resp is not a ref, should return a owned buffer.
    ///
    /// In order to return `Vec<u8>` from RpcSvrResp, should take the msg out, thus require task to
    /// by &mut, but this does not affect encoding.
    fn encode_resp<C: Codec>(
        codec: &C, task: Self::ChannelItem,
    ) -> (u64, Result<(Vec<u8>, Option<Buffer>), EncodedErr>);
}

/// A writer channel to send response to the server framework.
///
/// It can be cloned anywhere.
/// The user doesn't need to call it directly.
pub struct RespNoti<T: Send + 'static>(
    pub(crate) crossfire::MTx<Result<T, (u64, Option<RpcIntErr>)>>,
);

impl<T: Send + 'static> Clone for RespNoti<T> {
    #[inline]
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T: Send + 'static> RespNoti<T> {
    pub fn new(tx: crossfire::MTx<Result<T, (u64, Option<RpcIntErr>)>>) -> Self {
        Self(tx)
    }

    #[inline]
    pub fn done(self, task: T) {
        let _ = self.0.send(Ok(task));
    }

    #[inline]
    pub(crate) fn send_err(&self, seq: u64, err: Option<RpcIntErr>) -> Result<(), ()> {
        if self.0.send(Err((seq, err))).is_err() { return Err(()) } else { Ok(()) }
    }
}

/// Sum up trait for server response task
pub trait ServerTaskResp: ServerTaskEncode + Send + Sized + Unpin + 'static + fmt::Debug {}

/// How to decode a server request
pub trait ServerTaskDecode<R: Send + Unpin + 'static>: Send + Sized + Unpin + 'static {
    fn decode_req<'a, C: Codec>(
        codec: &'a C, action: RpcAction<'a>, seq: u64, req: &'a [u8], blob: Option<Buffer>,
        noti: RespNoti<R>,
    ) -> Result<Self, ()>;
}

/// How to encode a server response
pub trait ServerTaskEncode {
    fn encode_resp<C: Codec>(
        self, codec: &C,
    ) -> (u64, Result<(Vec<u8>, Option<Buffer>), EncodedErr>);
}

/// How to notify Rpc framework when a task is done
///
/// This is not mandatory for the framework, this a guideline,
/// You can skip this as long as you send the result back to RespNoti.
pub trait ServerTaskDone<T: Send + 'static, E: RpcErrCodec>: Sized + 'static {
    /// Should implement for enum delegation, not intended for user call
    fn _set_result(&mut self, res: Result<(), E>) -> RespNoti<T>;

    /// For users, set the result in the task and send it back
    #[inline]
    fn set_result(mut self, res: Result<(), E>)
    where
        T: std::convert::From<Self>,
    {
        // NOTE: To allow a trait to consume self, must require Sized
        let noti = self._set_result(res);
        let parent: T = self.into();
        noti.done(parent);
    }
}

/// Get RpcAction from a enum task, or a sub-type that fits multiple RpcActions
pub trait ServerTaskAction {
    fn get_action<'a>(&'a self) -> RpcAction<'a>;
}

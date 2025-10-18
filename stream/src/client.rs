//! The module contains traits defined for the client-side

use crate::client_timer::ClientTaskTimer;
use crate::proto::RpcAction;
use captains_log::filter::Filter;
use crossfire::MAsyncRx;
pub use occams_rpc_core::ClientConfig;
use occams_rpc_core::{
    Codec,
    error::{EncodedErr, RpcErrCodec, RpcError, RpcIntErr},
    runtime::AsyncIO,
};
use std::fmt;
use std::future::Future;
use std::io;
use std::ops::DerefMut;

/// A trait implemented by the user for the client-side, to define the customizable plugin.
pub trait ClientFactory: Send + Sync + Sized + 'static {
    /// A [captains-log::filter::Filter](https://docs.rs/captains-log/latest/captains_log/filter/index.html) implementation
    /// The type that new_logger returns.
    ///
    /// maybe a `Arc<LogFilter> or KeyFilter<Arc<LogFilter>>`
    type Logger: Filter + Send + 'static;

    /// Define the codec to serialization and deserialization
    ///
    /// Refers to [occams_rpc_core::Codec](https://docs.rs/occams-rpc-core/latest/occams_rpc_core/trait.Codec.html)
    type Codec: Codec;

    /// Define the RPC task from client-side
    ///
    /// Either one ClientTask or an enum of multiple ClientTask.
    /// If you have multiple task type, recommend to use the `enum_dispatch` crate.
    ///
    /// You can use [crate::macros::client_task_enum] and [crate::macros::client_task] on task type
    type Task: ClientTask;

    /// Define the transport layer protocol
    ///
    /// Refers to [ClientTransport]
    type Transport: ClientTransport<Self>;

    /// Define the adaptor of async runtime
    ///
    /// Refers to [occams_rpc_core::runtime::AsyncIO](https://docs.rs/occams-rpc-core/latest/occams_rpc_core/runtime/index.html)
    type IO: AsyncIO;

    /// Define how the async runtime spawn a task
    ///
    /// You may spawn with globally runtime, or to a owned runtime executor
    fn spawn_detach<F, R>(&self, f: F)
    where
        F: Future<Output = R> + Send + 'static,
        R: Send + 'static;

    /// You should keep clientConfig inside ServerFactory, get_config() will return the reference.
    fn get_config(&self) -> &ClientConfig;

    /// Construct a [captains_log::filter::Filter](https://docs.rs/captains-log/latest/captains_log/filter/trait.Filter.html) to oganize log of a client
    ///
    fn new_logger(&self, client_id: u64, server_id: u64) -> Self::Logger;
    /// TODO Fix the logger interface

    /// How to deal with error
    ///
    /// You can overwrite this to implement retry logic
    #[inline(always)]
    fn error_handle(&self, task: Self::Task) {
        task.done();
    }

    /// You can overwrite this to assign a client_id
    #[inline(always)]
    fn get_client_id(&self) -> u64 {
        0
    }
}

/// This trait is for client-side transport layer protocol.
///
/// The implementation can be found on:
///
/// - [occams-rpc-tcp](https://docs.rs/occams-rpc-tcp): For TCP and Unix socket
pub trait ClientTransport<F: ClientFactory>: fmt::Debug + Send + Sized + 'static {
    /// How to establish an async connection.
    fn connect(
        addr: &str, config: &ClientConfig, client_id: u64, server_id: u64, logger: F::Logger,
    ) -> impl Future<Output = Result<Self, RpcIntErr>> + Send;

    /// The ClientTransport holds a logger, the server will use it by reference.
    fn get_logger(&self) -> &F::Logger;

    /// Shutdown the write direction of the connection
    fn close_conn(&self) -> impl Future<Output = ()> + Send;

    /// Flush the request for the socket writer, if the transport has buffering logic
    fn flush_req(&self) -> impl Future<Output = io::Result<()>>;

    /// Write out the encoded request task
    fn write_req<'a>(
        &'a self, buf: &'a [u8], blob: Option<&'a [u8]>, need_flush: bool,
    ) -> impl Future<Output = io::Result<()>> + Send;

    /// Read the response and decode it from the socket, find and notify the registered ClientTask
    fn read_resp(
        &self, factory: &F, codec: &F::Codec, close_ch: Option<&MAsyncRx<()>>,
        task_reg: &mut ClientTaskTimer<F>,
    ) -> impl std::future::Future<Output = Result<bool, RpcIntErr>> + Send;
}

/// Sum up trait for client task, including request and response
pub trait ClientTask:
    ClientTaskAction
    + ClientTaskEncode
    + ClientTaskDecode
    + ClientTaskDone
    + DerefMut<Target = ClientTaskCommon>
    + Send
    + Sized
    + 'static
    + fmt::Debug
    + Unpin
{
}

/// Encode the request to buffer that can be send to server
pub trait ClientTaskEncode {
    /// sererialized the msg into buf (with std::io::Writer), and return the size written
    fn encode_req<C: Codec>(&self, codec: &C, buf: &mut Vec<u8>) -> Result<usize, ()>;

    /// Contain optional extra data to send to server side.
    ///
    /// By Default, return None when client task does not have a req_blob field
    #[inline(always)]
    fn get_req_blob(&self) -> Option<&[u8]> {
        None
    }
}

/// Decode the response from server and assign to the task struct
pub trait ClientTaskDecode {
    fn decode_resp<C: Codec>(&mut self, codec: &C, buf: &[u8]) -> Result<(), ()>;

    /// You can call crate::io::AllocateBuf::reserve(_size) on the following types:
    /// `Option<Vec<u8>>`, `Vec<u8>`, `Option<io_buffer::Buffer>`, `io_buffer::Buffer`
    ///
    /// By Default, return None when client task does not have a resp_blob field
    #[inline(always)]
    fn reserve_resp_blob(&mut self, _size: i32) -> Option<&mut [u8]> {
        None
    }
}

/// client_task_enum should impl this for user, not used by framework
pub trait ClientTaskGetResult<E: RpcErrCodec> {
    /// Check the result of the task
    fn get_result(&self) -> Result<(), &RpcError<E>>;
}

/// How to notify from Rpc framework to user when a task is done
///
/// The rpc framework first call set_ok or set_xxx_error, then call done
pub trait ClientTaskDone: Sized + 'static {
    /// Set the result.
    /// Called by RPC framework
    fn set_custom_error<C: Codec>(&mut self, codec: &C, e: EncodedErr);

    /// Called by RPC framework
    fn set_rpc_error(&mut self, e: RpcIntErr);

    fn set_ok(&mut self);

    fn done(self);
}

/// Get RpcAction from a enum task, or a sub-type that fits multiple RpcActions
pub trait ClientTaskAction {
    fn get_action<'a>(&'a self) -> RpcAction<'a>;
}

/// A common struct for every ClientTask
///
/// The fields might be extended in the future
#[derive(Debug, Default)]
pub struct ClientTaskCommon {
    /// Every task should be assigned an ID which is unique inside a socket connection
    pub seq: u64,
}

impl ClientTaskCommon {
    pub fn seq(&self) -> u64 {
        self.seq
    }
    pub fn set_seq(&mut self, seq: u64) {
        self.seq = seq;
    }
}

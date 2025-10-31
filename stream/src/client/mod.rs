//! The module contains traits defined for the client-side

pub use razor_rpc_core::ClientConfig;

pub mod task;
use task::{ClientTask, ClientTaskDone};

pub mod stream;
pub mod timer;
use timer::ClientTaskTimer;

mod pool;
pub use pool::ClientPool;
mod failover;
pub use failover::FailoverPool;

mod throttler;

use captains_log::filter::LogFilter;
use crossfire::MAsyncRx;
use razor_rpc_core::{Codec, error::RpcIntErr};
use std::future::Future;
use std::sync::Arc;
use std::{fmt, io};

/// A trait implemented by the user for the client-side, to define the customizable plugin.
///
/// # NOTE
///
/// If you choose implement this trait rather than use [ClientDefault].
/// We recommend your implementation to Deref<Target=orb::AsyncRuntime>
/// then the blanket trait in `orb::AsyncRuntime` will automatically impl AsyncRuntime on your ClientFacts type.
/// Refer to the code of [ClientDefault] for example.
pub trait ClientFacts: orb::AsyncRuntime + Send + Sync + Sized + 'static {
    /// Define the codec to serialization and deserialization
    ///
    /// Refers to [razor_rpc_core::Codec](https://docs.rs/razor-rpc-core/latest/razor_rpc_core/trait.Codec.html)
    type Codec: Codec;

    /// Define the RPC task from client-side
    ///
    /// Either one ClientTask or an enum of multiple ClientTask.
    /// If you have multiple task type, recommend to use the `enum_dispatch` crate.
    ///
    /// You can use macro [client_task_enum](crate::client::task::client_task_enum) and [client_task](crate::client::task::client_task) on task type
    type Task: ClientTask;

    /// You should keep ClientConfig inside, get_config() will return the reference.
    fn get_config(&self) -> &ClientConfig;

    /// Construct a [captains_log::filter::Filter](https://docs.rs/captains-log/latest/captains_log/filter/trait.Filter.html) to oganize log of a client
    fn new_logger(&self) -> Arc<LogFilter>;
    /// TODO Fix the logger interface

    /// How to deal with error
    ///
    /// The FailoverPool will overwrite this to implement retry logic
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

/// A trait to support sending request task in async text, for all router and connection pool
/// implementations
pub trait ClientCaller: Send {
    type Facts: ClientFacts;
    fn send_req(&self, task: <Self::Facts as ClientFacts>::Task)
    -> impl Future<Output = ()> + Send;
}

/// A trait to support sending request task in blocking text, for all router and connection pool
/// implementations
pub trait ClientCallerBlocking: Send {
    type Facts: ClientFacts;
    fn send_req_blocking(&self, task: <Self::Facts as ClientFacts>::Task);
}

impl<C: ClientCaller + Send + Sync> ClientCaller for Arc<C> {
    type Facts = C::Facts;
    #[inline(always)]
    async fn send_req(&self, task: <Self::Facts as ClientFacts>::Task) {
        self.as_ref().send_req(task).await
    }
}

impl<C: ClientCallerBlocking + Send + Sync> ClientCallerBlocking for Arc<C> {
    type Facts = C::Facts;

    #[inline(always)]
    fn send_req_blocking(&self, task: <Self::Facts as ClientFacts>::Task) {
        self.as_ref().send_req_blocking(task);
    }
}

/// This trait is for client-side transport layer protocol.
///
/// The implementation can be found on:
///
/// - [razor-rpc-tcp](https://docs.rs/razor-rpc-tcp): For TCP and Unix socket
///
/// # NOTE:
///
/// Instead of binding this to ClientFacts,
/// we use the associate type `RT` in generic param instead of ClientFacts to break cycle dep.
/// because [FailoverPool] will rewrap the facts into its own.
pub trait ClientTransport: fmt::Debug + Send + Sized + 'static {
    /// How to establish an async connection.
    ///
    /// conn_id: used for log fmt, can by the same of addr.
    fn connect(
        addr: &str, conn_id: &str, config: &ClientConfig,
    ) -> impl Future<Output = Result<Self, RpcIntErr>> + Send;

    /// Shutdown the write direction of the connection
    fn close_conn<F: ClientFacts>(&self, logger: &LogFilter) -> impl Future<Output = ()> + Send;

    /// Flush the request for the socket writer, if the transport has buffering logic
    fn flush_req<F: ClientFacts>(
        &self, logger: &LogFilter,
    ) -> impl Future<Output = io::Result<()>> + Send;

    /// Write out the encoded request task
    fn write_req<'a, F: ClientFacts>(
        &'a self, logger: &LogFilter, buf: &'a [u8], blob: Option<&'a [u8]>, need_flush: bool,
    ) -> impl Future<Output = io::Result<()>> + Send;

    /// Read the response and decode it from the socket, find and notify the registered ClientTask
    fn read_resp<F: ClientFacts>(
        &self, facts: &F, logger: &LogFilter, codec: &F::Codec, close_ch: Option<&MAsyncRx<()>>,
        task_reg: &mut ClientTaskTimer<F>,
    ) -> impl std::future::Future<Output = Result<bool, RpcIntErr>> + Send;
}

/// An example ClientFacts for general use
pub struct ClientDefault<T: ClientTask, RT: orb::AsyncRuntime, C: Codec> {
    pub logger: Arc<LogFilter>,
    config: ClientConfig,
    rt: RT,
    _phan: std::marker::PhantomData<fn(&C, &T)>,
}

impl<T: ClientTask, RT: orb::AsyncRuntime, C: Codec> ClientDefault<T, RT, C> {
    pub fn new(config: ClientConfig, rt: RT) -> Arc<Self> {
        Arc::new(Self { logger: Arc::new(LogFilter::new()), config, rt, _phan: Default::default() })
    }

    #[inline]
    pub fn set_log_level(&self, level: log::Level) {
        self.logger.set_level(level);
    }
}

impl<T: ClientTask, RT: orb::AsyncRuntime, C: Codec> std::ops::Deref for ClientDefault<T, RT, C> {
    type Target = RT;
    fn deref(&self) -> &Self::Target {
        &self.rt
    }
}

impl<T: ClientTask, RT: orb::AsyncRuntime, C: Codec> ClientFacts for ClientDefault<T, RT, C> {
    type Codec = C;
    type Task = T;

    #[inline]
    fn new_logger(&self) -> Arc<LogFilter> {
        self.logger.clone()
    }

    #[inline]
    fn get_config(&self) -> &ClientConfig {
        &self.config
    }
}

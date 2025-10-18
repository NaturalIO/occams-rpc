//! The RpcClient implementation
//!
//! [RpcClient] represents a client-side connection.
//!
//! On Drop, will close the connection on write-side, the response reader coroutine not exit
//! until all the ClientTask have a response or after `task_timeout` is reached.
//!
//! The user sends packets in sequence, with a throttler controlling the IO depth of in-flight packets.
//! An internal timer then registers the request through a channel, and when the response
//! is received, it can optionally notify the user through a user-defined channel or another mechanism.

use std::{
    cell::UnsafeCell,
    fmt,
    future::Future,
    mem::transmute,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    task::{Context, Poll},
};

use crate::client_timer::ClientTaskTimer;
use crate::{client::*, proto, throttler::*};
use crossfire::*;
use futures::pin_mut;
use occams_rpc_core::{error::*, runtime::*};
use std::time::Duration;
use sync_utils::{time::DelayedTime, waitgroup::WaitGroupGuard};

/// RpcClient represents a client-side connection.
///
/// On Drop, the connection will be closed on the write-side. The response reader coroutine will not exit
/// until all the ClientTasks have a response or after `task_timeout` is reached.
///
/// The user sends packets in sequence, with a throttler controlling the IO depth of in-flight packets.
/// An internal timer then registers the request through a channel, and when the response
/// is received, it can optionally notify the user through a user-defined channel or another mechanism.
pub struct RpcClient<F: ClientFactory> {
    close_tx: Option<MTx<()>>,
    inner: Arc<RpcClientInner<F>>,
}

impl<F: ClientFactory> RpcClient<F> {
    /// Make a streaming connection to the server, returns [RpcClient] on success
    #[inline]
    pub fn connect(
        factory: Arc<F>, addr: &str, server_id: u64, last_resp_ts: Option<Arc<AtomicU64>>,
    ) -> impl Future<Output = Result<Self, RpcIntErr>> + Send {
        async move {
            let client_id = factory.get_client_id();
            let logger = factory.new_logger(client_id, server_id);
            let conn = <F::Transport as ClientTransport<F>>::connect(
                addr,
                factory.get_config(),
                client_id,
                server_id,
                logger,
            )
            .await?;
            Ok(RpcClient::new(factory, conn, client_id, server_id, last_resp_ts))
        }
    }

    #[inline]
    fn new(
        factory: Arc<F>, conn: F::Transport, client_id: u64, server_id: u64,
        last_resp_ts: Option<Arc<AtomicU64>>,
    ) -> Self {
        let (_close_tx, _close_rx) = mpmc::unbounded_async::<()>();
        let inner = Arc::new(RpcClientInner::new(
            factory,
            conn,
            client_id,
            server_id,
            _close_rx,
            last_resp_ts,
        ));
        logger_debug!(inner.logger(), "{:?} connected", inner);
        let _inner = inner.clone();
        inner.factory.spawn_detach(async move {
            _inner.receive_loop().await;
        });
        Self { close_tx: Some(_close_tx), inner }
    }

    #[inline]
    pub fn get_codec(&self) -> &F::Codec {
        &self.inner.codec
    }

    /// Should be call in sender thread
    #[inline(always)]
    pub async fn ping(&self) -> Result<(), RpcIntErr> {
        self.inner.send_ping_req().await
    }

    #[inline(always)]
    pub fn get_last_resp_ts(&self) -> u64 {
        if let Some(ts) = self.inner.last_resp_ts.as_ref() { ts.load(Ordering::Relaxed) } else { 0 }
    }

    /// Since sender and receiver are two threads, might be close on either side
    #[inline(always)]
    pub fn is_closed(&self) -> bool {
        self.inner.closed.load(Ordering::SeqCst)
    }

    /// Force the receiver to exit.
    ///
    /// You can call it when connectivity probes detect that a server is unreachable.
    pub async fn set_error_and_exit(&self) {
        self.inner.has_err.store(true, Ordering::SeqCst);
        self.inner.conn.close_conn().await;
        if let Some(close_tx) = self.close_tx.as_ref() {
            let _ = close_tx.send(()); // This equals to RpcClient::drop
        }
    }

    /// send_task() should only be called without parallelism.
    ///
    /// NOTE: We define this to be an immutable function to avoid a borrow check, it might be changed to
    /// &mut self in the future.
    ///
    /// Since the transport layer might have buffer, user should always call flush explicitly.
    /// You can set `need_flush` = true for some urgent messages, or call flush_req() explicitly.
    #[inline(always)]
    pub async fn send_task(&self, task: F::Task, need_flush: bool) -> Result<(), RpcIntErr> {
        self.inner.send_task(task, need_flush).await
    }

    /// Since the transport layer might have buffer, user should always call flush explicitly.
    /// you can set `need_flush` = true for some urgent message, or call flush_req() explicitly.
    #[inline(always)]
    pub async fn flush_req(&self) -> Result<(), RpcIntErr> {
        self.inner.flush_req().await
    }

    /// Check the throttler and see if future send_task() might be blocked
    #[inline]
    pub fn will_block(&self) -> bool {
        if let Some(t) = self.inner.throttler.as_ref() { t.nearly_full() } else { false }
    }

    /// Wait for the response of in-flight tasks to be received
    #[inline(always)]
    pub async fn throttle(&self) -> bool {
        if self.inner.closed.load(Ordering::SeqCst) {
            return false;
        }
        if let Some(t) = self.inner.throttler.as_ref() {
            return t.throttle().await;
        } else {
            false
        }
    }
}

impl<F: ClientFactory> Drop for RpcClient<F> {
    fn drop(&mut self) {
        self.close_tx.take();
        let timer = self.inner.get_timer_mut();
        timer.stop_reg_task();
        self.inner.closed.store(true, Ordering::SeqCst);
    }
}

impl<F: ClientFactory> fmt::Debug for RpcClient<F> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.inner.fmt(f)
    }
}

struct RpcClientInner<F: ClientFactory> {
    client_id: u64,
    conn: F::Transport,
    seq: AtomicU64,
    close_rx: MAsyncRx<()>, // When RpcClient(sender) dropped, receiver will be timer
    closed: AtomicBool,     // flag set by either sender or receive on there exit
    timer: UnsafeCell<ClientTaskTimer<F>>,
    has_err: AtomicBool,
    throttler: Option<Throttler>,
    last_resp_ts: Option<Arc<AtomicU64>>,
    encode_buf: UnsafeCell<Vec<u8>>,
    codec: F::Codec,
    factory: Arc<F>,
}

unsafe impl<F: ClientFactory> Send for RpcClientInner<F> {}

unsafe impl<F: ClientFactory> Sync for RpcClientInner<F> {}

impl<F: ClientFactory> fmt::Debug for RpcClientInner<F> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.conn.fmt(f)
    }
}

impl<F: ClientFactory> RpcClientInner<F> {
    pub fn new(
        factory: Arc<F>, conn: F::Transport, client_id: u64, server_id: u64,
        close_rx: MAsyncRx<()>, last_resp_ts: Option<Arc<AtomicU64>>,
    ) -> Self {
        let config = factory.get_config();
        let thresholds = config.thresholds;
        let mut client_inner = Self {
            client_id,
            conn,
            close_rx,
            closed: AtomicBool::new(false),
            seq: AtomicU64::new(1),
            timer: UnsafeCell::new(ClientTaskTimer::new(
                server_id,
                client_id,
                config.task_timeout,
                thresholds,
            )),
            encode_buf: UnsafeCell::new(Vec::with_capacity(1024)),
            throttler: None,
            last_resp_ts,
            has_err: AtomicBool::new(false),
            codec: F::Codec::default(),
            factory,
        };
        if thresholds > 0 {
            logger_trace!(
                client_inner.logger(),
                "{:?} throttler is set to {}",
                client_inner,
                thresholds,
            );
            client_inner.throttler = Some(Throttler::new(thresholds));
        } else {
            logger_trace!(client_inner.logger(), "{:?} throttler is disabled", client_inner);
        }

        client_inner
    }

    #[inline(always)]
    fn logger(&self) -> &F::Logger {
        self.conn.get_logger()
    }

    #[inline(always)]
    fn get_timer_mut(&self) -> &mut ClientTaskTimer<F> {
        unsafe { transmute(self.timer.get()) }
    }

    #[inline(always)]
    fn get_encoded_buf(&self) -> &mut Vec<u8> {
        unsafe { transmute(self.encode_buf.get()) }
    }

    /// Directly work on the socket steam, when failed
    async fn send_task(&self, mut task: F::Task, need_flush: bool) -> Result<(), RpcIntErr> {
        let timer = self.get_timer_mut();
        timer.pending_task_count_ref().fetch_add(1, Ordering::SeqCst);
        if self.closed.load(Ordering::Acquire) {
            logger_warn!(
                self.logger(),
                "{:?} sending task {:?} failed: {}",
                self,
                task,
                RpcIntErr::IO,
            );
            task.set_rpc_error(RpcIntErr::IO);
            self.factory.error_handle(task);
            timer.pending_task_count_ref().fetch_sub(1, Ordering::SeqCst); // rollback
            return Err(RpcIntErr::IO);
        }

        match self.send_request(&mut task, need_flush).await {
            Err(e) => {
                logger_warn!(self.logger(), "{:?} sending task {:?} err: {}", self, task, e);
                // rollback counter
                timer.pending_task_count_ref().fetch_sub(1, Ordering::SeqCst);
                task.set_rpc_error(e);
                self.factory.error_handle(task);
                self.closed.store(true, Ordering::SeqCst);
                self.has_err.store(true, Ordering::SeqCst);
                timer.stop_reg_task();
                return Err(RpcIntErr::IO);
            }
            Ok(_) => {
                logger_trace!(self.logger(), "{:?} send task {:?} ok", self, task);
                // register task to norifier
                let mut wg: Option<WaitGroupGuard> = None;
                if let Some(throttler) = self.throttler.as_ref() {
                    wg = Some(throttler.add_task());
                }
                timer.reg_task(task, wg).await;
                return Ok(());
            }
        }
    }

    #[inline(always)]
    async fn flush_req(&self) -> Result<(), RpcIntErr> {
        if let Err(e) = self.conn.flush_req().await {
            logger_warn!(self.logger(), "{:?} flush_req flush err: {}", self, e);
            self.closed.store(true, Ordering::SeqCst);
            self.has_err.store(true, Ordering::SeqCst);
            let timer = self.get_timer_mut();
            timer.stop_reg_task();
            return Err(RpcIntErr::IO);
        }
        Ok(())
    }

    #[inline(always)]
    async fn send_request(&self, task: &mut F::Task, need_flush: bool) -> Result<(), RpcIntErr> {
        let seq = self.seq_update();
        task.set_seq(seq);
        let buf = self.get_encoded_buf();
        match proto::ReqHead::encode(&self.codec, buf, self.client_id, task) {
            Err(_) => {
                logger_warn!(self.logger(), "{:?} send_req encode req {:?} err", self, task);
                return Err(RpcIntErr::Encode);
            }
            Ok(blob_buf) => {
                if let Err(e) = self.conn.write_req(buf, blob_buf, need_flush).await {
                    logger_warn!(
                        self.logger(),
                        "{:?} send_req write req {:?} err: {:?}",
                        self,
                        task,
                        e
                    );
                    self.closed.store(true, Ordering::SeqCst);
                    self.has_err.store(true, Ordering::SeqCst);
                    let timer = self.get_timer_mut();
                    // TODO check stop_reg_task
                    timer.stop_reg_task();
                    return Err(RpcIntErr::IO);
                }
                return Ok(());
            }
        }
    }

    #[inline(always)]
    async fn send_ping_req(&self) -> Result<(), RpcIntErr> {
        if self.closed.load(Ordering::Acquire) {
            logger_warn!(self.logger(), "{:?} send_ping_req skip as conn closed", self);
            return Err(RpcIntErr::IO);
        }
        let buf = self.get_encoded_buf();
        proto::ReqHead::encode_ping(buf, self.client_id, self.seq_update());
        // Ping does not need to reg_task, and have no error_handle, just to keep the connection
        // alive. Connection Prober can monitor the liveness of ClientConn
        if let Err(e) = self.conn.write_req(buf, None, true).await {
            logger_warn!(self.logger(), "{:?} send ping err: {:?}", self, e);
            self.closed.store(true, Ordering::SeqCst);
            return Err(RpcIntErr::IO);
        }
        Ok(())
    }

    // return Ok(false) when close_rx has close and nothing more pending resp to receive
    async fn recv_some(&self) -> Result<(), RpcIntErr> {
        for _ in 0i32..20 {
            // Underlayer rpc socket is buffered, might not yeal to runtime
            // return if recv_one_resp runs too long, allow timer to be fire at each second
            match self.recv_one_resp().await {
                Err(e) => {
                    return Err(e);
                }
                Ok(_) => {
                    if let Some(last_resp_ts) = self.last_resp_ts.as_ref() {
                        last_resp_ts.store(DelayedTime::get(), Ordering::Relaxed);
                    }
                }
            }
        }
        Ok(())
    }

    async fn recv_one_resp(&self) -> Result<(), RpcIntErr> {
        let timer = self.get_timer_mut();
        loop {
            if self.closed.load(Ordering::Acquire) {
                logger_trace!(
                    self.conn.get_logger(),
                    "{:?} read_resp from already close",
                    self.conn
                );
                // ensure task receive on normal exit
                if timer.check_pending_tasks_empty() || self.has_err.load(Ordering::Relaxed) {
                    return Err(RpcIntErr::IO);
                }
                if let Err(_e) = self.conn.read_resp(&self.factory, &self.codec, None, timer).await
                {
                    self.closed.store(true, Ordering::SeqCst);
                    return Err(RpcIntErr::IO);
                }
            } else {
                // Block here for new header without timeout
                match self
                    .conn
                    .read_resp(&self.factory, &self.codec, Some(&self.close_rx), timer)
                    .await
                {
                    Err(_e) => {
                        return Err(RpcIntErr::IO);
                    }
                    Ok(r) => {
                        // TODO FIXME
                        if !r {
                            self.closed.store(true, Ordering::SeqCst);
                            continue;
                        }
                    }
                }
            }
        }
    }

    async fn receive_loop(&self) {
        let mut tick = <F::IO as AsyncIO>::tick(Duration::from_secs(1));
        loop {
            let f = self.recv_some();
            pin_mut!(f);
            let selector = ReceiverTimerFuture::new(self, &mut tick, &mut f);
            match selector.await {
                Ok(_) => {}
                Err(e) => {
                    logger_debug!(self.logger(), "{:?} receive_loop error: {}", self, e);
                    self.closed.store(true, Ordering::SeqCst);
                    let timer = self.get_timer_mut();
                    timer.clean_pending_tasks(self.factory.as_ref());
                    // If pending_task_count > 0 means some tasks may still remain in the pending chan
                    while timer.pending_task_count_ref().load(Ordering::SeqCst) > 0 {
                        // After the 'closed' flag has taken effect,
                        // pending_task_count will not keep growing,
                        // so there is no need to sleep here.
                        timer.clean_pending_tasks(self.factory.as_ref());
                        <F::IO as AsyncIO>::sleep(Duration::from_secs(1)).await;
                    }
                    return;
                }
            }
        }
    }

    // Adjust the waiting queue
    fn time_reach(&self) {
        if let Some(throttler) = self.throttler.as_ref() {
            logger_trace!(
                self.logger(),
                "{:?} has {} pending_tasks",
                self,
                throttler.get_inflight_tasks_count()
            );
        }
        let timer = self.get_timer_mut();
        timer.adjust_task_queue(self.factory.as_ref());
        return;
    }

    #[inline(always)]
    fn seq_update(&self) -> u64 {
        self.seq.fetch_add(1, Ordering::SeqCst)
    }
}

impl<F: ClientFactory> Drop for RpcClientInner<F> {
    fn drop(&mut self) {
        let timer = self.get_timer_mut();
        timer.clean_pending_tasks(self.factory.as_ref());
    }
}

struct ReceiverTimerFuture<'a, F, P>
where
    F: ClientFactory,
    P: Future<Output = Result<(), RpcIntErr>> + Unpin,
{
    client: &'a RpcClientInner<F>,
    inv: Pin<&'a mut <F::IO as AsyncIO>::Interval>,
    recv_future: Pin<&'a mut P>,
}

impl<'a, F, P> ReceiverTimerFuture<'a, F, P>
where
    F: ClientFactory,
    P: Future<Output = Result<(), RpcIntErr>> + Unpin,
{
    fn new(
        client: &'a RpcClientInner<F>, inv: &'a mut <F::IO as AsyncIO>::Interval,
        recv_future: &'a mut P,
    ) -> Self {
        Self { inv: Pin::new(inv), client, recv_future: Pin::new(recv_future) }
    }
}

// Return Ok(true) to indicate Ok
// Return Ok(false) when client sender has close normally
// Err(e) when connection error
impl<'a, F, P> Future for ReceiverTimerFuture<'a, F, P>
where
    F: ClientFactory,
    P: Future<Output = Result<(), RpcIntErr>> + Unpin,
{
    type Output = Result<(), RpcIntErr>;

    fn poll(self: Pin<&mut Self>, ctx: &mut Context) -> Poll<Self::Output> {
        let mut _self = self.get_mut();
        //let _inv = Pin::new(<<_self.inv as F::IO as AsyncIO>::Interval as TimeInterval>);
        // In case ticker not fire, and ensure ticker schedule after ready
        while let Poll::Ready(_) = _self.inv.as_mut().poll_tick(ctx) {
            _self.client.time_reach();
        }
        if _self.client.has_err.load(Ordering::Relaxed) {
            // When sentinel detect peer unreachable, recv_some mighe blocked, at least inv will
            // wait us, just exit
            return Poll::Ready(Err(RpcIntErr::IO));
        }
        _self.client.get_timer_mut().poll_sent_task(ctx);
        // Even if receive future has block, we should poll_sent_task in order to detect timeout event
        if let Poll::Ready(r) = _self.recv_future.as_mut().poll(ctx) {
            return Poll::Ready(r);
        }
        return Poll::Pending;
    }
}

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

use super::{notifier::*, proto::*, task::*, throttler::*};
use crate::{UnifyBufStream, UnifyStream};
use crate::{config::*, error::*};
use bytes::BytesMut;
use captains_log::LogFilter;
use crossfire::{SendError, mpsc};
use futures::{future::FutureExt, pin_mut};
use io_engine::buffer::Buffer;
use nix::errno::Errno;
use sync_utils::{time::DelayedTime, waitgroup::WaitGroupGuard};
use tokio::time::{Duration, Instant, Interval, interval_at, sleep};
use zerocopy::AsBytes;

macro_rules! retry_with_err {
    ($self:expr, $t:expr, $err:expr) => {
        if let Some(retry_task_sender) = $self.retry_task_sender.as_ref() {
            let retry_task = RetryTaskInfo { task: $t, task_err: $err.clone() };
            if let Err(SendError(rt)) = retry_task_sender.send(Some(retry_task)) {
                rt.unwrap().task.set_result(Err($err));
            }
        }
    };
}

/// RpcClient represents a client-side connection. connection will close on dropped
pub struct RpcClient<T: RpcClientTask + Send + Unpin + 'static> {
    close_h: Option<mpsc::TxUnbounded<()>>,
    inner: Arc<RpcClientInner<T>>,
}

impl<T> RpcClient<T>
where
    T: RpcClientTask + Send + Unpin + 'static,
{
    /// timeout_setting: only use read_timeout/write_timeout
    pub fn new(
        server_id: u64, client_id: u64, stream: UnifyStream, config: RPCConfig,
        retry_tx: Option<mpsc::TxUnbounded<Option<RetryTaskInfo<T>>>>,
        last_resp_ts: Option<Arc<AtomicU64>>, logger: Option<Arc<LogFilter>>,
    ) -> Self {
        let (_close_tx, _close_rx) = mpsc::unbounded_future::<()>();
        Self {
            close_h: Some(_close_tx),
            inner: Arc::new(RpcClientInner::new(
                server_id,
                client_id,
                stream,
                retry_tx,
                _close_rx,
                config,
                last_resp_ts,
                logger,
            )),
        }
    }

    pub fn start_receiver(&self) {
        let inner = self.inner.clone();
        tokio::spawn(async move {
            inner.receive_loop().await;
        });
    }

    /// Should be call in sender thread
    #[inline(always)]
    pub async fn ping(&self) -> Result<(), RpcError> {
        self.inner.send_ping_req().await
    }

    #[inline(always)]
    pub fn get_last_resp_ts(&self) -> u64 {
        if let Some(ts) = self.inner.last_resp_ts.as_ref() { ts.load(Ordering::Acquire) } else { 0 }
    }

    /// Since sender and receiver are two threads, might be close on either side
    #[inline(always)]
    pub fn is_closed(&self) -> bool {
        self.inner.closed.load(Ordering::Acquire)
    }

    /// Force the receiver to exit
    pub async fn set_error_and_exit(&self) {
        self.inner.has_err.store(true, Ordering::Release);
        let stream = self.inner.get_stream_mut();
        let _ = stream.close().await; // stream close is just shutdown on sending, receiver might not be notified on peer dead
        if let Some(close_h) = self.close_h.as_ref() {
            let _ = close_h.send(()); // This equals to RpcClient::drop
        }
    }

    #[inline(always)]
    pub async fn send_task(&self, task: T, need_flush: bool) -> Result<(), RpcError> {
        self.inner.send_task(task, need_flush).await
    }

    #[inline(always)]
    pub async fn flush_req(&self) -> Result<(), RpcError> {
        self.inner.flush_req().await
    }

    #[inline]
    pub fn will_block(&self) -> bool {
        if let Some(t) = self.inner.throttler.as_ref() { t.nearly_full() } else { false }
    }

    #[inline(always)]
    pub async fn throttle(&self) -> bool {
        if self.inner.closed.load(Ordering::Acquire) {
            return false;
        }
        if let Some(t) = self.inner.throttler.as_ref() {
            return t.throttle().await;
        } else {
            false
        }
    }
}

impl<T> Drop for RpcClient<T>
where
    T: RpcClientTask + Send + Unpin + 'static,
{
    fn drop(&mut self) {
        self.close_h.take();
        let notifier = self.inner.get_notifier_mut();
        notifier.stop_reg_task();
        self.inner.closed.store(true, Ordering::Release);
    }
}

impl<T> fmt::Debug for RpcClient<T>
where
    T: RpcClientTask + Send + Unpin + 'static,
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.inner.fmt(f)
    }
}

struct RpcClientInner<T>
where
    T: RpcClientTask + Send + Unpin + 'static,
{
    server_id: u64,
    client_id: u64,
    stream: UnsafeCell<UnifyBufStream>,
    timeout: TimeoutSetting,
    seq: AtomicU64,
    retry_task_sender: Option<mpsc::TxUnbounded<Option<RetryTaskInfo<T>>>>,
    close_h: mpsc::RxUnbounded<()>, // When RpcClient(sender) dropped, receiver will be notifier
    closed: AtomicBool,             // flag set by either sender or receive on there exit
    notifier: UnsafeCell<RpcClientTaskNotifier<T>>,
    has_err: AtomicBool,
    resp_buf: UnsafeCell<BytesMut>,
    throttler: Option<Throttler>,
    last_resp_ts: Option<Arc<AtomicU64>>,
    logger: Arc<LogFilter>,
}

unsafe impl<T> Send for RpcClientInner<T> where T: RpcClientTask + Send + Unpin + 'static {}

unsafe impl<T> Sync for RpcClientInner<T> where T: RpcClientTask + Send + Unpin + 'static {}

impl<T> fmt::Debug for RpcClientInner<T>
where
    T: RpcClientTask + Send + Unpin + 'static,
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "rpc client {}:{}", self.server_id, self.client_id)
    }
}

impl<T> RpcClientInner<T>
where
    T: RpcClientTask + Send + Unpin + 'static,
{
    pub fn new(
        server_id: u64, client_id: u64, stream: UnifyStream,
        retry_tx: Option<mpsc::TxUnbounded<Option<RetryTaskInfo<T>>>>,
        close_h: mpsc::RxUnbounded<()>, config: RPCConfig, last_resp_ts: Option<Arc<AtomicU64>>,
        mut logger: Option<Arc<LogFilter>>,
    ) -> Self {
        if let None = logger {
            logger = Some(Arc::new(LogFilter::new()));
            logger.as_mut().unwrap().set_level(log::Level::Trace);
        }

        let mut client_inner = Self {
            server_id,
            client_id,
            stream: UnsafeCell::new(UnifyBufStream::with_capacity(33 * 1024, 33 * 1024, stream)),
            retry_task_sender: retry_tx,
            close_h,
            closed: AtomicBool::new(false),
            seq: AtomicU64::new(1),
            timeout: config.timeout,
            notifier: UnsafeCell::new(RpcClientTaskNotifier::new(
                server_id,
                client_id,
                config.timeout.task_timeout,
                config.thresholds,
            )),
            resp_buf: UnsafeCell::new(BytesMut::with_capacity(512)),
            throttler: None,
            last_resp_ts,
            has_err: AtomicBool::new(false),
            logger: logger.unwrap(),
        };

        if config.thresholds > 0 {
            logger_trace!(
                client_inner.logger,
                "{:?} throttler is set to {}",
                client_inner,
                config.thresholds,
            );
            client_inner.throttler = Some(Throttler::new(config.thresholds));
        } else {
            logger_trace!(client_inner.logger, "{:?} throttler is disabled", client_inner,);
        }

        client_inner
    }

    #[inline(always)]
    fn get_stream_mut(&self) -> &mut UnifyBufStream {
        unsafe { std::mem::transmute(self.stream.get()) }
    }

    #[inline(always)]
    fn get_notifier_mut(&self) -> &mut RpcClientTaskNotifier<T> {
        unsafe { std::mem::transmute(self.notifier.get()) }
    }

    #[inline(always)]
    fn get_resp_buf(&self, len: usize) -> &mut BytesMut {
        let buf: &mut BytesMut = unsafe { transmute(self.resp_buf.get()) };
        buf.resize(len as usize, 0);
        buf
    }

    //    #[inline(always)]
    //    fn should_close(&self, e: Errno) -> bool {
    //          TODO replace this
    //        e == Errno::EAGAIN || e == Errno::EHOSTDOWN
    //    }

    /// Directly work on the socket steam, when failed
    async fn send_task(&self, mut task: T, need_flush: bool) -> Result<(), RpcError> {
        let notifier = self.get_notifier_mut();
        notifier.pending_task_count_ref().fetch_add(1, Ordering::SeqCst);
        if self.closed.load(Ordering::Acquire) {
            logger_warn!(
                self.logger,
                "{:?} sending task {} failed: {}",
                self,
                task,
                RPC_ERR_CLOSED,
            );
            retry_with_err!(self, task, RPC_ERR_CLOSED);
            notifier.pending_task_count_ref().fetch_sub(1, Ordering::SeqCst); // rollback
            return Err(RPC_ERR_COMM);
        }

        match self.send_request(&mut task, need_flush).await {
            Err(e) => {
                logger_warn!(self.logger, "{:?} sending task {} failed: {:?}", self, task, e);
                notifier.pending_task_count_ref().fetch_sub(1, Ordering::SeqCst); // rollback
                retry_with_err!(self, task, e.clone());
                self.closed.store(true, Ordering::Release);
                self.has_err.store(true, Ordering::Release);
                notifier.stop_reg_task();
                return Err(e);
            }
            Ok(_) => {
                logger_trace!(self.logger, "{:?} send task {} success", self, task);
                // register task to norifier
                let mut wg: Option<WaitGroupGuard> = None;
                if let Some(throttler) = self.throttler.as_ref() {
                    wg = Some(throttler.add_task());
                }
                notifier.reg_task(task, wg).await;
                return Ok(());
            }
        }
    }

    #[inline(always)]
    async fn flush_req(&self) -> Result<(), RpcError> {
        let writer = self.get_stream_mut();
        let r = writer.flush_timeout(self.timeout.write_timeout).await;
        if r.is_err() {
            logger_warn!(self.logger, "{:?} flush_req flush err: {:?}", self, r);
            self.closed.store(true, Ordering::Release);
            self.has_err.store(true, Ordering::Release);
            let notifier = self.get_notifier_mut();
            notifier.stop_reg_task();

            return Err(RPC_ERR_COMM);
        }
        Ok(())
    }

    #[inline(always)]
    async fn send_request(&self, task: &mut T, need_flush: bool) -> Result<(), RpcError> {
        let seq = self.seq_update();
        task.set_seq(seq);
        let (header, action_str, msg_buf, blob_buf) = ReqHead::encode(self.client_id, task);
        let writer = self.get_stream_mut();
        let header_bytes = header.as_bytes();
        let mut data_len = header_bytes.len();
        if let Err(e) = writer.write_timeout(header_bytes, self.timeout.write_timeout).await {
            logger_warn!(self.logger, "{:?} send_req write req header err: {:?}", self, e);
            return Err(RPC_ERR_COMM);
        }
        if let Some(action_s) = action_str {
            data_len += action_s.len();
            if let Err(e) = writer.write_timeout(action_s, self.timeout.write_timeout).await {
                logger_warn!(self.logger, "{:?} send_req write req header err: {:?}", self, e);
                return Err(RPC_ERR_COMM);
            }
        }
        if let Some(msg) = msg_buf {
            data_len += msg.len();
            if let Err(e) = writer.write_timeout(msg.as_bytes(), self.timeout.write_timeout).await {
                logger_warn!(self.logger, "{:?} send_req write req header err: {:?}", self, e);
                return Err(RPC_ERR_COMM);
            }
        }
        if let Some(blob) = blob_buf {
            data_len += blob.len();
            if let Err(e) = writer.write_timeout(blob.as_bytes(), self.timeout.write_timeout).await
            {
                logger_warn!(self.logger, "{:?} send_req write req ext err: {:?}", self, e);
                return Err(RPC_ERR_COMM);
            }
        }
        if need_flush || data_len >= 32 * 1024 {
            let r = writer.flush_timeout(self.timeout.write_timeout).await;
            if r.is_err() {
                warn!("{:?} send_req flush req err: {:?}", self, r);
                return Err(RPC_ERR_COMM);
            }
        }

        return Ok(());
    }

    // return Ok(false) when close_h has close and nothing more pending resp to receive
    async fn recv_some(&self) -> Result<(), RpcError> {
        for _ in 0i32..20 {
            // Underlayer rpc socket is buffered, might not yeal to runtime
            // return if recv_one_resp runs too long, allow timer to be fire at each second
            match self.recv_one_resp().await {
                Err(e) => {
                    //if e == RPC_ERR_CLOSED {
                    //    return Ok(false);
                    //} else {
                    return Err(e);
                    //}
                }
                Ok(_) => {
                    if let Some(last_resp_ts) = self.last_resp_ts.as_ref() {
                        last_resp_ts.store(DelayedTime::get(), Ordering::Release);
                    }
                }
            }
        }
        Ok(())
    }

    async fn _recv_and_dump(&self, l: usize) -> Result<(), RpcError> {
        let reader = self.get_stream_mut();
        match Buffer::alloc(l) {
            Err(_) => {
                self.closed.store(true, Ordering::Release);
                logger_warn!(self.logger, "{:?} alloc buf failed", self);
                return Err(RPC_ERR_COMM);
            }
            Ok(mut buf) => {
                if let Err(e) = reader.read_exact_timeout(&mut buf, self.timeout.read_timeout).await
                {
                    logger_warn!(self.logger, "{:?} recv task failed: {:?}", self, e);
                    return Err(RPC_ERR_COMM);
                }
                return Ok(());
            }
        }
    }

    async fn _recv_error(&self, resp_head: &RespHead, task: T) -> Result<(), RpcError> {
        log_debug_assert!(resp_head.flag > 0);
        let reader = self.get_stream_mut();
        match resp_head.flag {
            1 => {
                let rpc_err = RpcError::Posix(Errno::from_raw(resp_head.msg_len as i32));
                //if self.should_close(err_no) {
                //    self.closed.store(true, Ordering::Release);
                //    retry_with_err!(self, task, rpc_err);
                //    return Err(RPC_ERR_COMM);
                //} else {
                retry_with_err!(self, task, rpc_err);
                return Ok(());
            }
            2 => {
                let buf = self.get_resp_buf(resp_head.blob_len as usize);
                match reader.read_exact_timeout(buf, self.timeout.read_timeout).await {
                    Err(e) => {
                        logger_warn!(self.logger, "{:?} recv buffer error: {:?}", self, e);
                        self.closed.store(true, Ordering::Release);
                        retry_with_err!(self, task, RPC_ERR_COMM);
                        return Err(RPC_ERR_COMM);
                    }
                    Ok(_) => {
                        match str::from_utf8(buf) {
                            Err(_) => {
                                logger_error!(
                                    self.logger,
                                    "{:?} recv task {} err string invalid",
                                    self,
                                    task
                                );
                                retry_with_err!(self, task, RPC_ERR_DECODE);
                            }
                            Ok(s) => {
                                let rpc_err = RpcError::Remote(s.to_string());
                                retry_with_err!(self, task, rpc_err);
                            }
                        }
                        return Ok(());
                    }
                }
            }
            _ => unreachable!(),
        }
    }

    async fn _recv_resp_body(&self, resp_head: &RespHead) -> Result<(), RpcError> {
        let reader = self.get_stream_mut();
        let read_timeout = self.timeout.read_timeout;
        let notifier = self.get_notifier_mut();
        let blob_len = resp_head.blob_len;
        let read_buf = self.get_resp_buf(resp_head.msg_len as usize);
        if let Some(mut task_item) = notifier.take_task(resp_head.seq).await {
            let mut task = task_item.task.take().unwrap();
            if resp_head.flag > 0 {
                return self._recv_error(resp_head, task).await;
            }
            if resp_head.msg_len > 0 {
                if let Err(_) = reader.read_exact_timeout(read_buf, read_timeout).await {
                    self.closed.store(true, Ordering::Release);
                    retry_with_err!(self, task, RPC_ERR_COMM);
                    return Err(RPC_ERR_COMM);
                }
            } // When msg_len == 0, read_buf has 0 size
            if blob_len > 0 {
                match task.get_resp_ext_buf_mut(blob_len as i32) {
                    None => {
                        logger_error!(
                            self.logger,
                            "{:?} rpc client task {} has no ext_buf",
                            self,
                            task,
                        );
                        task.set_result(Err(RPC_ERR_DECODE));
                        return self._recv_and_dump(blob_len as usize).await;
                    }
                    Some(ext_buf) => {
                        // Should ensure ext_buf has len meat blob_len
                        if let Err(e) = reader.read_exact_timeout(ext_buf, read_timeout).await {
                            logger_warn!(
                                self.logger,
                                "{:?} rpc client reader read ext_buf err: {:?}",
                                self,
                                e
                            );
                            retry_with_err!(self, task, RPC_ERR_COMM);
                            return Err(RPC_ERR_COMM);
                        }
                    }
                }
            }
            logger_debug!(self.logger, "{:?} recv task {} ok", self, task);
            // set result of task, and notify task completed
            task.set_result(Ok(&read_buf));
            return Ok(());
        } else {
            let seq = resp_head.seq;
            logger_debug!(self.logger, "{:?} notifier take_task(seq={}) return None", self, seq);
            let mut data_len = 0;
            if resp_head.flag == 0 {
                data_len += resp_head.msg_len + resp_head.blob_len;
            } else if resp_head.flag == RESP_FLAG_HAS_ERR_STRING {
                data_len += resp_head.blob_len;
            }
            return self._recv_and_dump(data_len as usize).await;
        }
    }

    async fn recv_one_resp(&self) -> Result<(), RpcError> {
        let mut resp_head_buf = [0u8; RPC_RESP_HEADER_LEN];
        let reader = self.get_stream_mut();
        let read_timeout = self.timeout.read_timeout;

        'HeaderLoop: loop {
            if self.closed.load(Ordering::Acquire) {
                let notifier = self.get_notifier_mut();
                // ensure task receive on normal exit
                if notifier.check_pending_tasks_empty() || self.has_err.load(Ordering::Acquire) {
                    return Err(RPC_ERR_CLOSED);
                }

                if let Err(_e) = reader.read_exact_timeout(&mut resp_head_buf, read_timeout).await {
                    logger_debug!(
                        self.logger,
                        "{:?} rpc client read resp head when closing err: {:?}",
                        self,
                        _e
                    );
                    return Err(RPC_ERR_COMM);
                }
                break;
            } else {
                // Block here for new header without timeout
                let close_f = self.close_h.recv().fuse();
                pin_mut!(close_f);
                let read_header_f = reader.read_exact(&mut resp_head_buf).fuse();
                pin_mut!(read_header_f);
                futures::select! {
                    r = read_header_f => {
                        match r {
                            Err(_e) => {
                                logger_debug!(self.logger, "{:?} rpc client read resp head err: {:?}", self, _e);
                                return Err(RPC_ERR_COMM);
                            }
                            Ok(_) => {
                                break 'HeaderLoop;
                            },
                        }
                    },
                    _ = close_f => {
                        self.closed.store(true, Ordering::Release);
                        continue
                    }
                }
            }
        }
        match RespHead::decode(&resp_head_buf) {
            Err(_e) => {
                logger_debug!(
                    self.logger,
                    "{:?} rpc client decode_response_header err: {:?}",
                    self,
                    _e
                );
                return Err(RPC_ERR_COMM);
            }
            Ok(head) => {
                logger_trace!(self.logger, "{:?} rpc client read head response {}", self, head);
                return self._recv_resp_body(head).await;
            }
        }
    }

    async fn receive_loop(&self) {
        let later = Instant::now() + Duration::from_secs(1);
        let mut tick = Box::pin(interval_at(later, Duration::from_secs(1)));
        loop {
            let f = self.recv_some();
            pin_mut!(f);
            let selector = ReciverTimerFuture::new(self, &mut tick, &mut f);
            match selector.await {
                Ok(_) => {}
                Err(e) => {
                    logger_debug!(self.logger, "{:?} receive_loop error: {:?}", self, e);
                    self.closed.store(true, Ordering::Release);
                    let notifier = self.get_notifier_mut();
                    notifier.clean_pending_tasks(self.retry_task_sender.as_ref());
                    // If pending_task_count > 0 means some tasks may still remain in the pending chan
                    while notifier.pending_task_count_ref().load(Ordering::SeqCst) > 0 {
                        // After the 'closed' flag has taken effect,
                        // pending_task_count will not keep growing,
                        // so there is no need to sleep here.
                        notifier.clean_pending_tasks(self.retry_task_sender.as_ref());
                        sleep(Duration::from_secs(1)).await;
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
                self.logger,
                "{:?} has {} pending_tasks",
                self,
                throttler.get_inflight_tasks_count()
            );
        }
        let notifier = self.get_notifier_mut();
        notifier.adjust_task_queue(self.retry_task_sender.as_ref());
        return;
    }

    #[inline(always)]
    fn seq_update(&self) -> u64 {
        self.seq.fetch_add(1, Ordering::SeqCst)
    }

    #[inline(always)]
    async fn send_ping_req(&self) -> Result<(), RpcError> {
        if self.closed.load(Ordering::Acquire) {
            logger_warn!(self.logger, "{:?} send_ping_req skip as conn closed", self);
            return Err(RPC_ERR_CLOSED);
        }
        // encode response header
        let header = ReqHead {
            magic: RPC_MAGIC,
            seq: self.seq_update(),
            client_id: self.client_id,
            ver: 1,
            format: 0,
            action: PING_ACTION,
            msg_len: 0 as u32,
            blob_len: 0 as u32,
        };
        let write_timeout = self.timeout.write_timeout;
        let writer = self.get_stream_mut();
        if let Err(e) = writer.write_timeout(header.as_bytes(), write_timeout).await {
            logger_warn!(self.logger, "{:?} send_ping_req write head {:?}", self, e);
            self.closed.store(true, Ordering::Release);
            return Err(RPC_ERR_COMM);
        }

        if let Err(e) = writer.flush_timeout(write_timeout).await {
            logger_warn!(self.logger, "{:?} send_ping_req flush req err: {:?}", self, e);
            self.closed.store(true, Ordering::Release);
            return Err(RPC_ERR_COMM);
        }

        logger_trace!(self.logger, "{:?} rpc client send ping request: {}", self, header);
        return Ok(());
    }
}

impl<T> Drop for RpcClientInner<T>
where
    T: RpcClientTask + Send + Unpin + 'static,
{
    fn drop(&mut self) {
        let notifier = self.get_notifier_mut();
        notifier.clean_pending_tasks(self.retry_task_sender.as_ref());
    }
}

struct ReciverTimerFuture<'a, T, F>
where
    T: RpcClientTask + Send + Unpin + 'static,
    F: Future<Output = Result<(), RpcError>> + Unpin,
{
    client: &'a RpcClientInner<T>,
    inv: &'a mut Pin<Box<Interval>>,
    recv_future: Pin<&'a mut F>,
}

impl<'a, T, F> ReciverTimerFuture<'a, T, F>
where
    T: RpcClientTask + Send + Unpin + 'static,
    F: Future<Output = Result<(), RpcError>> + Unpin,
{
    fn new(
        client: &'a RpcClientInner<T>, inv: &'a mut Pin<Box<Interval>>, recv_future: &'a mut F,
    ) -> Self {
        Self { inv, client, recv_future: Pin::new(recv_future) }
    }
}

// Return Ok(true) to indicate Ok
// Return Ok(false) when client sender has close normally
// Err(e) when connection error
impl<'a, T, F> Future for ReciverTimerFuture<'a, T, F>
where
    T: RpcClientTask + Send + Unpin,
    F: Future<Output = Result<(), RpcError>> + Unpin,
{
    type Output = Result<(), RpcError>;

    fn poll(self: Pin<&mut Self>, ctx: &mut Context) -> Poll<Self::Output> {
        let mut _self = self.get_mut();
        // In case ticker not fire, and ensure ticker schedule after ready
        while let Poll::Ready(_) = _self.inv.as_mut().poll_tick(ctx) {
            _self.client.time_reach();
        }
        if _self.client.has_err.load(Ordering::Acquire) {
            // When sentinel detect peer unreachable, recv_some mighe blocked, at least inv will
            // wait us, just exit
            return Poll::Ready(Err(RPC_ERR_CLOSED));
        }
        _self.client.get_notifier_mut().poll_sent_task(ctx);
        // Even if receive future has block, we should poll_sent_task in order to detect timeout event
        if let Poll::Ready(r) = _self.recv_future.as_mut().poll(ctx) {
            return Poll::Ready(r);
        }
        return Poll::Pending;
    }
}

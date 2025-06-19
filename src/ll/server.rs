use std::{
    cell::UnsafeCell,
    fmt,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use crate::{UnifyBufStream, UnifyListener, UnifyStream};
use bytes::BytesMut;
use crossfire::mpsc::{RxUnbounded, TxUnbounded, unbounded_future};
use futures::{
    FutureExt,
    future::{AbortHandle, Abortable},
};
use zerocopy::AsBytes;

use super::proto::*;
use super::task::*;
use crate::config::TimeoutSetting;
use crate::error::*;
use captains_log::LogFilter;
use io_engine::buffer::Buffer;

#[async_trait]
pub trait ServerConnFactory: Clone + Sync + Send + 'static {
    fn serve_conn(&self, reader: RpcSvrReader);

    async fn exit_conns(&mut self) {}
}

pub struct RpcServer<F>
where
    F: ServerConnFactory,
{
    listeners_abort: Vec<(AbortHandle, String)>,
    timeout: TimeoutSetting,
    factory: F,
    conn_ref_count: Arc<AtomicU64>,
    logger: Arc<LogFilter>,
}

impl<F> RpcServer<F>
where
    F: ServerConnFactory,
{
    pub fn new(factory: F, timeout_setting: TimeoutSetting, logger: Arc<LogFilter>) -> Self {
        Self {
            listeners_abort: Vec::new(),
            timeout: timeout_setting,
            factory,
            conn_ref_count: Arc::new(AtomicU64::new(0)),
            logger,
        }
    }

    pub fn listen(&mut self, rt: &tokio::runtime::Runtime, listeners: Vec<UnifyListener>) {
        for mut l in listeners {
            let (abort_handle, abort_registration) = AbortHandle::new_pair();
            let timeouts = self.timeout;
            let factory = self.factory.clone();
            let conn_ref_count = self.conn_ref_count.clone();
            let listener_info = format!("listener:{}", l);
            let logger = self.logger.clone();
            let abrt = Abortable::new(
                async move {
                    logger_debug!(logger, "listening on {}", l);
                    loop {
                        match l.accept().await {
                            Err(e) => {
                                logger_warn!(logger, "{} accept error: {}", l, e);
                                return;
                            }
                            Ok(stream) => {
                                let (reader, writer) = RpcSvrConnInner::new(
                                    stream,
                                    timeouts,
                                    conn_ref_count.clone(),
                                    logger.clone(),
                                );
                                factory.serve_conn(reader);
                                tokio::spawn(async move {
                                    writer.serve().await;
                                });
                            }
                        }
                    }
                },
                abort_registration,
            )
            .map(|x| match x {
                Ok(_) => {}
                Err(e) => {
                    warn!("rpc server exit listening accept as {:?}", e);
                }
            });
            rt.spawn(abrt);
            self.listeners_abort.push((abort_handle, listener_info));
        }
    }

    pub async fn close(&mut self) {
        // close listeners
        for h in &self.listeners_abort {
            h.0.abort();
            logger_info!(self.logger, "{} has closed", h.1);
        }
        // close current all qtxs and notify client redirect connection
        self.factory.exit_conns().await;
        // wait client close all connections
        let mut wait_exit_count = 0;
        loop {
            wait_exit_count += 1;
            if wait_exit_count > 90 {
                logger_warn!(
                    self.logger,
                    "closed as wait too long for all conn closed voluntarily({} conn left)",
                    self.conn_ref_count.load(Ordering::Acquire)
                );
                break;
            }
            if self.conn_ref_count.load(Ordering::Acquire) == 0 {
                logger_info!(self.logger, "closed as conn_ref_count is 0");
                break;
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }
}

/// A writer channel to send reponse. Can be clone anywhere.
#[derive(Clone)]
pub struct RpcSvrRespWriter(TxUnbounded<RpcSvrResp>);

impl RpcSvrRespWriter {
    #[inline]
    pub fn done(&self, resp: RpcSvrResp) {
        let _ = self.0.send(resp);
    }
}

pub struct RpcSvrReader {
    inner: Arc<RpcSvrConnInner>,
    action_buf: BytesMut,
    msg_buf: BytesMut,
    done_tx: TxUnbounded<RpcSvrResp>,
}

impl fmt::Display for RpcSvrReader {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "RpcConn {}", self.inner.get_stream())
    }
}

impl RpcSvrReader {
    /// Get a writer for asynchronous response
    #[inline(always)]
    pub fn get_resp_writer(&self) -> RpcSvrRespWriter {
        RpcSvrRespWriter(self.done_tx.clone())
    }

    /// recv_req and return a temporary structure.
    ///
    /// NOTE: you should consume the buffer ref before recv another request.
    pub async fn recv_req<'a>(&'a mut self) -> Result<RpcSvrReq<'a>, RpcError> {
        let inner = self.inner.as_ref();
        let reader = inner.get_stream_mut();
        let read_timeout = inner.timeouts.read_timeout;
        let mut req_header_buf = [0u8; RPC_REQ_HEADER_LEN];
        loop {
            {
                if let Err(_e) = reader
                    .read_exact_timeout(&mut req_header_buf, inner.timeouts.idle_timeout)
                    .await
                {
                    logger_trace!(inner.logger, "{}: read timeout", self);
                    return Err(RPC_ERR_TIMEOUT);
                }
            }
            let rpc_head: &ReqHead;
            match ReqHead::decode(&req_header_buf) {
                Err(_) => {
                    logger_warn!(inner.logger, "{}: decode_header error", self);
                    return Err(RPC_ERR_COMM);
                }
                Ok(head) => {
                    rpc_head = head;
                }
            }
            logger_trace!(inner.logger, "{}: recv req: {}", self, rpc_head);
            if rpc_head.action == 0 && rpc_head.msg_len == 0 && rpc_head.blob_len == 0 {
                // Ping message, send response ASAP
                let _ = self.done_tx.send(RpcSvrResp { seq: rpc_head.seq, res: Ok((None, None)) });
                // Receive another
                continue;
            }
            let action = match rpc_head.get_action() {
                Ok(num) => RpcAction::Num(num),
                Err(action_len) => {
                    self.action_buf.resize(action_len as usize, 0);
                    match reader.read_exact_timeout(&mut self.action_buf, read_timeout).await {
                        Err(_) => {
                            logger_trace!(inner.logger, "{}: read_exact error", self);
                            return Err(RPC_ERR_COMM);
                        }
                        Ok(_) => {
                            match std::str::from_utf8(&self.action_buf) {
                                Ok(s) => RpcAction::Str(s),
                                Err(_) => {
                                    error!("{}: read action string decode error", self);
                                    return Err(RPC_ERR_DECODE);
                                    // XXX stop reading or consume junk data?
                                }
                            }
                        }
                    }
                }
            };

            let mut msg: Option<&[u8]> = None;
            if rpc_head.msg_len > 0 {
                self.msg_buf.resize(rpc_head.msg_len as usize, 0);
                match reader.read_exact_timeout(&mut self.msg_buf, read_timeout).await {
                    Err(_) => {
                        logger_trace!(inner.logger, "{}: read_exact error", self);
                        return Err(RPC_ERR_COMM);
                    }
                    Ok(_) => {
                        msg = Some(&self.msg_buf);
                    }
                }
            }

            let mut blob: Option<Buffer> = None;
            if rpc_head.blob_len > 0 {
                match Buffer::alloc(rpc_head.blob_len as usize) {
                    Err(_) => return Err(RPC_ERR_ENCODE),
                    Ok(mut ext_buf) => {
                        match reader.read_exact_timeout(&mut ext_buf, read_timeout).await {
                            Err(_) => {
                                logger_trace!(inner.logger, "{}: read_exact_buffer error", self);
                                return Err(RPC_ERR_COMM);
                            }
                            Ok(_) => {
                                blob = Some(ext_buf);
                            }
                        }
                    }
                }
            }
            return Ok(RpcSvrReq::<'a> { seq: rpc_head.seq, action, msg, blob });
        }
    }
}

struct RpcSvrWriter {
    inner: Arc<RpcSvrConnInner>,
    done_rx: RxUnbounded<RpcSvrResp>,
}

impl fmt::Display for RpcSvrWriter {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "RpcConn {}", self.inner.get_stream())
    }
}

impl RpcSvrWriter {
    async fn serve(mut self) {
        while let Ok(resp) = self.done_rx.recv().await {
            if self.send_resp(&resp).await.is_err() {
                break;
            }
            while let Ok(resp) = self.done_rx.try_recv() {
                if self.send_resp(&resp).await.is_err() {
                    break;
                }
            }
            if self.flush_resp().await.is_err() {
                break;
            }
        }
        // Exit
        logger_trace!(self.inner.logger, "{} RpcSvrWriter exiting", self);
        self.close_conn().await;
    }

    #[inline(always)]
    async fn flush_resp(&mut self) -> Result<(), RpcError> {
        let inner = self.inner.as_ref();
        let writer = inner.get_stream_mut();
        let write_timeout = inner.timeouts.write_timeout;
        let r = writer.flush_timeout(write_timeout).await;
        if r.is_err() {
            logger_debug!(inner.logger, "{}: flush err: {:?}", self, r);
            return Err(RPC_ERR_COMM);
        }
        logger_trace!(inner.logger, "{}: send_resp flushed", self);
        return Ok(());
    }

    /// Send response hold by `task_resp` (which is a temporary structure).
    #[inline]
    async fn send_resp<'a>(&'a mut self, task_resp: &'a RpcSvrResp) -> Result<(), RpcError> {
        let inner = self.inner.as_ref();
        let writer = inner.get_stream_mut();
        let write_timeout = inner.timeouts.write_timeout;
        let (header, msg_buf, blob_buf) = RespHead::encode(task_resp);
        if let Err(e) = writer.write_timeout(header.as_bytes(), write_timeout).await {
            logger_warn!(inner.logger, "{}: send_resp write resp header err: {:?}", self, e);
            return Err(RPC_ERR_COMM);
        }
        logger_trace!(inner.logger, "{}: send resp: {}", self, header);
        if let Some(msg) = msg_buf.as_ref() {
            if let Err(e) = writer.write_timeout(msg.as_bytes(), write_timeout).await {
                logger_debug!(inner.logger, "{}: send_resp write resp msg err: {:?}", self, e);
                return Err(RPC_ERR_COMM);
            }
        }
        if let Some(blob) = blob_buf.as_ref() {
            if let Err(e) = writer.write_timeout(blob.as_bytes(), write_timeout).await {
                logger_debug!(inner.logger, "{}: send_resp write resp blob err: {:?}", self, e);
                return Err(RPC_ERR_COMM);
            }
        }
        return Ok(());
    }

    async fn close_conn(self) {
        let inner = self.inner.as_ref();
        let writer = inner.get_stream_mut();
        let _ = writer.close().await;
    }
}

struct RpcSvrConnInner {
    stream: UnsafeCell<UnifyBufStream>,
    timeouts: TimeoutSetting,
    conn_ref_count: Arc<AtomicU64>,
    logger: Arc<LogFilter>,
}

unsafe impl Send for RpcSvrConnInner {}
unsafe impl Sync for RpcSvrConnInner {}

impl Drop for RpcSvrConnInner {
    fn drop(&mut self) {
        let ref_count = self.conn_ref_count.fetch_sub(1, Ordering::SeqCst);
        logger_debug!(
            self.logger,
            "RpcSvrConn {:?} droped, refcount turned to : {}",
            self.stream,
            ref_count
        );
    }
}

impl RpcSvrConnInner {
    fn new(
        stream: UnifyStream, timeouts: TimeoutSetting, conn_ref_count: Arc<AtomicU64>,
        logger: Arc<LogFilter>,
    ) -> (RpcSvrReader, RpcSvrWriter) {
        conn_ref_count.fetch_add(1, Ordering::SeqCst);
        let inner = Arc::new(Self {
            stream: UnsafeCell::new(UnifyBufStream::with_capacity(33 * 1024, 33 * 1024, stream)),
            timeouts,
            conn_ref_count,
            logger,
        });
        let (done_tx, done_rx) = unbounded_future::<RpcSvrResp>();
        let reader = RpcSvrReader {
            inner: inner.clone(),
            action_buf: BytesMut::with_capacity(128),
            msg_buf: BytesMut::with_capacity(512),
            done_tx,
        };
        let writer = RpcSvrWriter { inner, done_rx };
        return (reader, writer);
    }
}

impl RpcSvrConnInner {
    #[inline(always)]
    fn get_stream_mut(&self) -> &mut UnifyBufStream {
        unsafe { std::mem::transmute(self.stream.get()) }
    }

    #[inline(always)]
    fn get_stream(&self) -> &UnifyBufStream {
        unsafe { std::mem::transmute(self.stream.get()) }
    }
}

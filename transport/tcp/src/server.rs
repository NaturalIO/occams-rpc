use occams_rpc_core::io::{AsyncBufStream, AsyncRead, AsyncWrite, Cancellable, io_with_timeout};
use occams_rpc_core::runtime::AsyncIO;
use occams_rpc_core::{Codec, ServerConfig, error::*};
use occams_rpc_stream::server::{RpcSvrReq, ServerFactory, ServerTaskEncode, ServerTransport};
use occams_rpc_stream::{proto, proto::RpcAction};

use crate::net::{UnifyListener, UnifyStream};
use io_buffer::Buffer;
use std::cell::UnsafeCell;
use std::mem::transmute;
use std::sync::Arc;
use std::time::Duration;
use std::{fmt, io};

pub const SERVER_DEFAULT_BUF_SIZE: usize = 8 * 1024;

pub struct TcpServer<F: ServerFactory> {
    stream: UnsafeCell<AsyncBufStream<UnifyStream<F::IO>>>,
    _conn_count: Arc<()>,
    config: ServerConfig,
    /// for read
    action_buf: UnsafeCell<Vec<u8>>,
    /// for read
    msg_buf: UnsafeCell<Vec<u8>>,
    /// for write
    encode_buf: UnsafeCell<Vec<u8>>,
    logger: F::Logger,
}

unsafe impl<F: ServerFactory> Send for TcpServer<F> {}

unsafe impl<F: ServerFactory> Sync for TcpServer<F> {}

impl<F: ServerFactory> TcpServer<F> {
    // Because async runtimes does not support splitting read and write to static handler,
    // we use unsafe to achieve such goal,
    #[inline(always)]
    fn get_stream_mut(&self) -> &mut AsyncBufStream<UnifyStream<F::IO>> {
        unsafe { transmute(self.stream.get()) }
    }

    #[inline(always)]
    fn get_msg_buf(&self) -> &mut Vec<u8> {
        unsafe { transmute(self.msg_buf.get()) }
    }

    #[inline(always)]
    fn get_action_buf(&self) -> &mut Vec<u8> {
        unsafe { transmute(self.action_buf.get()) }
    }

    #[inline(always)]
    fn get_encode_buf(&self) -> &mut Vec<u8> {
        unsafe { transmute(self.encode_buf.get()) }
    }
}

impl<F: ServerFactory> fmt::Debug for TcpServer<F> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.get_stream_mut().fmt(f)
    }
}

impl<F: ServerFactory> ServerTransport<F> for TcpServer<F> {
    type Listener = UnifyListener<F::IO>;

    fn new_conn(stream: UnifyStream<F::IO>, factory: &F, conn_count: Arc<()>) -> Self {
        let config = factory.get_config();
        let logger = factory.new_logger();
        let mut buf_size = config.stream_buf_size;
        if buf_size == 0 {
            buf_size = SERVER_DEFAULT_BUF_SIZE;
        }
        Self {
            stream: UnsafeCell::new(AsyncBufStream::new(stream, buf_size)),
            config: config.clone(),
            action_buf: UnsafeCell::new(Vec::with_capacity(128)),
            msg_buf: UnsafeCell::new(Vec::with_capacity(512)),
            // TODO add const assert with RPC_RESP_HEADER_LEN
            encode_buf: UnsafeCell::new(Vec::with_capacity(512)),
            _conn_count: conn_count,
            logger,
        }
    }

    #[inline(always)]
    fn get_logger(&self) -> &F::Logger {
        &self.logger
    }

    /// recv_req and return a temporary structure.
    ///
    /// NOTE: you should consume the buffer ref before recv another request.
    async fn read_req<'a>(
        &'a self, close_ch: &crossfire::MAsyncRx<()>,
    ) -> Result<RpcSvrReq<'a>, RpcIntErr> {
        let reader = self.get_stream_mut();
        let read_timeout = self.config.read_timeout;
        let idle_timeout = self.config.idle_timeout;
        let mut req_header_buf = [0u8; proto::RPC_REQ_HEADER_LEN];

        let cancel_f = close_ch.recv_with_timer(F::IO::sleep(idle_timeout));
        match Cancellable::new(reader.read_exact(&mut req_header_buf), cancel_f).await {
            Ok(Err(e)) => {
                logger_debug!(self.logger, "{:?}: recv_req: err {}", self, e);
                return Err(RpcIntErr::IO);
            }
            Err(()) => {
                logger_trace!(self.logger, "{:?}: read timeout", self);
                return Err(RpcIntErr::Timeout);
            }
            _ => {}
        }
        let rpc_head: &proto::ReqHead;
        match proto::ReqHead::decode_head(&req_header_buf) {
            Err(e) => {
                logger_warn!(self.logger, "{:?}: decode_head error, {}", self, e);
                return Err(RpcIntErr::Decode);
            }
            Ok(head) => {
                rpc_head = head;
            }
        }
        logger_trace!(self.logger, "{:?}: recv req: {}", self, rpc_head);
        // XXX: we do return ping
        let action = match rpc_head.get_action() {
            Ok(num) => RpcAction::Num(num),
            Err(action_len) => {
                let action_buf = self.get_action_buf();
                action_buf.resize(action_len as usize, 0);
                match io_with_timeout!(F::IO, read_timeout, reader.read_exact(action_buf)) {
                    Err(e) => {
                        logger_trace!(self.logger, "{:?}: read_exact error {}", self, e);
                        return Err(RpcIntErr::IO);
                    }
                    Ok(_) => {
                        match std::str::from_utf8(action_buf) {
                            Ok(s) => RpcAction::Str(s),
                            Err(_) => {
                                error!("{:?}: read action string decode error", self);
                                return Err(RpcIntErr::Decode);
                                // XXX stop reading or consume junk data?
                            }
                        }
                    }
                }
            }
        };

        let msg_buf = self.get_msg_buf();
        msg_buf.resize(rpc_head.msg_len.get() as usize, 0);
        if rpc_head.msg_len > 0 {
            if let Err(e) = io_with_timeout!(F::IO, read_timeout, reader.read_exact(msg_buf)) {
                logger_trace!(self.logger, "{:?}: read req msg error: {:?}", self, e);
                return Err(RpcIntErr::IO);
            }
        }
        let mut blob: Option<Buffer> = None;
        let blob_len = rpc_head.blob_len.get() as i32;
        if blob_len > 0 {
            match Buffer::alloc(blob_len) {
                Err(_) => return Err(RpcIntErr::Decode),
                Ok(mut ext_buf) => {
                    match io_with_timeout!(F::IO, read_timeout, reader.read_exact(&mut ext_buf)) {
                        Err(e) => {
                            logger_trace!(
                                self.logger,
                                "{:?}: read_exact_buffer error: {}",
                                self,
                                e
                            );
                            return Err(RpcIntErr::IO);
                        }
                        Ok(_) => {
                            blob = Some(ext_buf);
                        }
                    }
                }
            }
        }
        return Ok(RpcSvrReq::<'a> { seq: rpc_head.seq.get(), action, msg: msg_buf, blob });
    }

    #[inline]
    async fn write_resp<C: Codec, T: ServerTaskEncode>(
        &self, codec: &C, mut task: T,
    ) -> io::Result<()> {
        let writer = self.get_stream_mut();
        let write_timeout = self.config.write_timeout;
        let buf = self.get_encode_buf();
        let (seq, blob_buf) = proto::RespHead::encode(&self.logger, codec, buf, &mut task);
        if let Err(e) = io_with_timeout!(F::IO, write_timeout, writer.write_all(buf)) {
            logger_warn!(
                self.logger,
                "{:?}: send_resp write resp seq={} msg err: {}",
                self,
                seq,
                e
            );
            return Err(e);
        }
        if let Some(blob) = blob_buf {
            if let Err(e) = io_with_timeout!(F::IO, write_timeout, writer.write_all(blob.as_ref()))
            {
                logger_debug!(
                    self.logger,
                    "{:?}: send_resp write resp seq={} blob err: {}",
                    self,
                    seq,
                    e
                );
                return Err(e);
            }
        }
        logger_trace!(self.logger, "{:?}: send resp seq={}", self, seq);
        return Ok(());
    }

    #[inline(always)]
    async fn write_resp_internal(&self, seq: u64, err: Option<RpcIntErr>) -> io::Result<()> {
        let writer = self.get_stream_mut();
        let write_timeout = self.config.write_timeout;
        let buf = self.get_encode_buf();
        let seq = proto::RespHead::encode_internal(&self.logger, buf, seq, err);
        if let Err(e) = io_with_timeout!(F::IO, write_timeout, writer.write_all(buf)) {
            logger_warn!(
                self.logger,
                "{:?}: send_resp write resp seq={} msg err: {}",
                self,
                seq,
                e
            );
            return Err(e);
        }
        logger_trace!(self.logger, "{:?}: send resp seq={}", self, seq);
        return Ok(());
    }

    #[inline(always)]
    async fn flush_resp(&self) -> io::Result<()> {
        let writer = self.get_stream_mut();
        if let Err(e) = io_with_timeout!(F::IO, self.config.write_timeout, writer.flush()) {
            logger_warn!(self.logger, "{:?}: flush err: {}", self, e);
            return Err(e);
        }
        logger_trace!(self.logger, "{:?}: flush_resp ok", self);
        return Ok(());
    }

    #[inline]
    async fn close_conn(&self) {
        if self.flush_resp().await.is_ok() {
            let writer = self.get_stream_mut();
            let _ = writer.get_inner().shutdown_write().await;
        }
    }
}

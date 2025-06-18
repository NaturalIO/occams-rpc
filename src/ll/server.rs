use std::{
    cell::UnsafeCell,
    fmt,
    mem::transmute,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use crate::{UnifyBufStream, UnifyListener, UnifyStream};
use bytes::BytesMut;
use futures::{
    FutureExt,
    future::{AbortHandle, Abortable},
};

use super::proto::*;
use super::task::*;
use crate::config::TimeoutSetting;
use crate::error::*;
use io_engine::buffer::Buffer;
use zerocopy::AsBytes;

#[async_trait]
pub trait RpcServerConnFactory: Clone + Sync + Send + 'static {
    fn serve_conn(&self, conn: RpcServerConn);

    async fn exit_conns(&mut self) {}
}

pub struct RpcServer<F>
where
    F: RpcServerConnFactory,
{
    listeners_abort: Vec<(AbortHandle, String)>,
    timeout: TimeoutSetting,
    factory: F,
    conn_ref_count: Arc<AtomicU64>,
}

impl<F> RpcServer<F>
where
    F: RpcServerConnFactory,
{
    pub fn new(factory: F, timeout_setting: TimeoutSetting) -> Self {
        Self {
            listeners_abort: Vec::new(),
            timeout: timeout_setting,
            factory,
            conn_ref_count: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn listen(&mut self, rt: &tokio::runtime::Runtime, listeners: Vec<UnifyListener>) {
        for mut l in listeners {
            let (abort_handle, abort_registration) = AbortHandle::new_pair();
            let timeouts = self.timeout;
            let factory = self.factory.clone();
            let conn_ref_count = self.conn_ref_count.clone();
            let listener_info = format!("listener:{}", l);
            let abrt = Abortable::new(
                async move {
                    debug!("listening on {}", l);
                    loop {
                        match l.accept().await {
                            Err(e) => {
                                warn!("{} accept error: {}", l, e);
                                return;
                            }
                            Ok(stream) => {
                                factory.serve_conn(RpcServerConn::new(
                                    stream,
                                    timeouts,
                                    conn_ref_count.clone(),
                                ));
                            }
                        }
                    }
                },
                abort_registration,
            )
            .map(|x| match x {
                Ok(_) => {}
                Err(e) => {
                    debug!("rpc server exit listening accept as {:?}", e);
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
            info!("{} has closed", h.1);
        }
        // close current all qtxs and notify client redirect connection
        self.factory.exit_conns().await;
        // wait client close all connections
        let mut wait_exit_count = 0;
        loop {
            wait_exit_count += 1;
            if wait_exit_count > 90 {
                warn!(
                    "closed as wait too long for all conn closed voluntarily({} conn left)",
                    self.conn_ref_count.load(Ordering::Acquire)
                );
                break;
            }
            if self.conn_ref_count.load(Ordering::Acquire) == 0 {
                info!("closed as conn_ref_count is 0");
                break;
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }
}

pub struct RpcServerConn(Arc<RpcConnInner>);

impl Clone for RpcServerConn {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl fmt::Display for RpcServerConn {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "RpcConn {}", self.0.get_stream())
    }
}

impl RpcServerConn {
    pub fn new(
        stream: UnifyStream, timeouts: TimeoutSetting, conn_ref_count: Arc<AtomicU64>,
    ) -> Self {
        Self(Arc::new(RpcConnInner::new(
            // set buf size to 33k so that one msg will hold in fuffer
            stream,
            timeouts,
            conn_ref_count,
        )))
    }

    pub async fn recv_req<Decoder, Req>(&self, decoder: &Decoder) -> Result<Req, RPCError>
    where
        Decoder: Fn(u64, u8, Option<&[u8]>, Option<Buffer>) -> Result<Req, RPCError>,
        Req: 'static,
    {
        let inner = self.0.as_ref();

        let reader = inner.get_stream_mut();
        let read_timeout = inner.timeouts.read_timeout;
        let mut req_header_buf = [0u8; RPC_REQ_HEADER_LEN];

        let mut read_buf: &mut BytesMut = unsafe { transmute(inner.req_buf.get()) };
        let rpc_head: &ReqHead;
        if let Err(_e) =
            reader.read_exact_timeout(&mut req_header_buf, inner.timeouts.idle_timeout).await
        {
            trace!("{}: read timeout", self);
            return Err(RPC_ERR_COMM);
        }
        match ReqHead::decode(&req_header_buf) {
            Err(_) => {
                trace!("{}: decode_header error", self);
                return Err(RPC_ERR_COMM);
            }
            Ok(head) => {
                rpc_head = head;
            }
        }
        trace!("{}: recv req: {}", self, rpc_head);

        let mut msg: Option<&[u8]> = None;
        if rpc_head.msg_len > 0 {
            read_buf.resize(rpc_head.msg_len as usize, 0);
            match reader.read_exact_timeout(&mut read_buf, read_timeout).await {
                Err(_) => {
                    trace!("{}: read_exact error", self);
                    return Err(RPC_ERR_COMM);
                }
                Ok(_) => {
                    msg = Some(&read_buf);
                }
            }
        }

        let mut extension: Option<Buffer> = None;
        if rpc_head.blob_len > 0 {
            match Buffer::alloc(rpc_head.blob_len as usize) {
                Err(e) => return Err(RPC_ERR_ENCODE),
                Ok(mut ext_buf) => {
                    match reader.read_exact_timeout(&mut ext_buf, read_timeout).await {
                        Err(_) => {
                            trace!("{}: read_exact_buffer error", self);
                            return Err(RPC_ERR_COMM);
                        }
                        Ok(_) => {
                            extension = Some(ext_buf);
                        }
                    }
                }
            }
        }
        decoder(rpc_head.seq, rpc_head.action, msg, extension)
    }

    #[inline(always)]
    pub async fn flush_resp(&self) -> Result<(), RPCError> {
        let inner = self.0.as_ref();
        let writer = inner.get_stream_mut();
        let write_timeout = inner.timeouts.write_timeout;
        let r = writer.flush_timeout(write_timeout).await;
        if r.is_err() {
            debug!("{}: flush err: {:?}", self, r);
            return Err(RPC_ERR_COMM);
        }
        trace!("{}: send_resp flushed", self);
        return Ok(());
    }

    pub async fn send_resp<'a>(
        &'a self, task_resp: &'a RpcResponse<'a>, need_flush: bool,
    ) -> Result<(), RPCError> {
        let inner = self.0.as_ref();

        let writer = inner.get_stream_mut();
        let write_timeout = inner.timeouts.write_timeout;

        let msg_len = if let Some(ref msg_buf) = task_resp.msg { msg_buf.len() } else { 0 };

        let blob_len = if let Some(ref ext_buf) = task_resp.extension { ext_buf.len() } else { 0 };

        let header = RespHead {
            magic: RPC_MAGIC,
            seq: task_resp.seq,
            format: 0,
            action: task_resp.action,
            err_no: task_resp.err_no,
            ver: 1,
            msg_len: msg_len as u32,
            blob_len: blob_len as u32,
            ..Default::default()
        };
        trace!("{}: send resp: {}", self, header);
        let r = writer.write_timeout(header.as_bytes(), write_timeout).await;
        if r.is_err() {
            warn!("{}: send_resp write resp header err: {:?}", self, r);
            return Err(RPC_ERR_COMM);
        }

        if let Some(msg_buf) = task_resp.msg.as_ref() {
            let r = writer.write_timeout(msg_buf, write_timeout).await;
            if r.is_err() {
                debug!("{}: send_resp write resp msg err: {:?}", self, r);
                return Err(RPC_ERR_COMM);
            }
        }

        if blob_len > 0 {
            let r =
                writer.write_timeout(task_resp.extension.as_ref().unwrap(), write_timeout).await;
            if r.is_err() {
                debug!("{}: send_resp write resp extension err: {:?}", self, r);
                return Err(RPC_ERR_COMM);
            }
        }
        if need_flush {
            let r = writer.flush_timeout(write_timeout).await;
            if r.is_err() {
                debug!("{}: send_resp flush resp err: {:?}", self, r);
                return Err(RPC_ERR_COMM);
            }
            trace!("{}: send_resp flushed resp: {}", self, header);
        }

        return Ok(());
    }

    pub async fn close_conn(&self) {
        let inner = self.0.as_ref();
        let writer = inner.get_stream_mut();
        let _ = writer.close().await;
    }
}

struct RpcConnInner {
    stream: UnsafeCell<UnifyBufStream>,
    timeouts: TimeoutSetting,
    req_buf: UnsafeCell<BytesMut>,
    conn_ref_count: Arc<AtomicU64>,
}

unsafe impl Send for RpcConnInner {}
unsafe impl Sync for RpcConnInner {}

impl Drop for RpcConnInner {
    fn drop(&mut self) {
        let ref_count = self.conn_ref_count.fetch_sub(1, Ordering::SeqCst);
        debug!("RpcConnInner {:?} droped, conn refcount turned to : {}", self.stream, ref_count);
    }
}

impl RpcConnInner {
    pub fn new(
        stream: UnifyStream, timeouts: TimeoutSetting, conn_ref_count: Arc<AtomicU64>,
    ) -> Self {
        conn_ref_count.fetch_add(1, Ordering::SeqCst);
        Self {
            stream: UnsafeCell::new(UnifyBufStream::with_capacity(33 * 1024, 33 * 1024, stream)),
            timeouts,
            req_buf: UnsafeCell::new(BytesMut::with_capacity(512)),
            conn_ref_count,
        }
    }
}

impl RpcConnInner {
    #[inline(always)]
    fn get_stream_mut(&self) -> &mut UnifyBufStream {
        unsafe { std::mem::transmute(self.stream.get()) }
    }

    #[inline(always)]
    fn get_stream(&self) -> &UnifyBufStream {
        unsafe { std::mem::transmute(self.stream.get()) }
    }
}

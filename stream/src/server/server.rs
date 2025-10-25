use crate::proto::RpcAction;
use crate::server::*;
use futures::future::{AbortHandle, Abortable};
use occams_rpc_core::{error::*, io::AsyncListener, runtime::AsyncIO};
use std::io;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// An RpcServer that listen, accept, and server connections, according to ServerFacts interface.
pub struct RpcServer<F>
where
    F: ServerFacts,
{
    listeners_abort: Vec<(AbortHandle, String)>,
    logger: F::Logger,
    facts: Arc<F>,
    conn_ref_count: Arc<()>,
    server_close_tx: Mutex<Option<crossfire::MTx<()>>>,
    server_close_rx: crossfire::MAsyncRx<()>,
}

impl<F> RpcServer<F>
where
    F: ServerFacts,
{
    pub fn new(facts: Arc<F>) -> Self {
        let (tx, rx) = crossfire::mpmc::unbounded_async();
        Self {
            listeners_abort: Vec::new(),
            logger: facts.new_logger(),
            facts,
            conn_ref_count: Arc::new(()),
            server_close_tx: Mutex::new(Some(tx)),
            server_close_rx: rx,
        }
    }

    pub fn listen<T: ServerTransport<F::IO>>(&mut self, addr: &str) -> io::Result<String> {
        match <T::Listener as AsyncListener>::bind(addr) {
            Err(e) => {
                error!("bind addr {:?} err: {}", addr, e);
                return Err(e);
            }
            Ok(mut listener) => {
                let local_addr = match listener.local_addr() {
                    Ok(addr) => addr,
                    Err(e) => {
                        if e.kind() == std::io::ErrorKind::AddrNotAvailable {
                            // For Unix sockets, return a dummy address
                            "0.0.0.0:0".parse().unwrap()
                        } else {
                            return Err(e);
                        }
                    }
                };
                let (abort_handle, abort_registration) = AbortHandle::new_pair();
                let facts = self.facts.clone();
                let conn_ref_count = self.conn_ref_count.clone();
                let listener_info = format!("listener {:?}", addr);
                let server_close_rx = self.server_close_rx.clone();
                debug!("listening on {:?}", listener);
                let abrt = Abortable::new(
                    async move {
                        loop {
                            match listener.accept().await {
                                Err(e) => {
                                    warn!("{:?} accept error: {}", listener, e);
                                    return;
                                }
                                Ok(stream) => {
                                    let conn = T::new_conn(
                                        stream,
                                        facts.get_config(),
                                        conn_ref_count.clone(),
                                    );
                                    Self::server_conn::<T>(conn, &facts, server_close_rx.clone())
                                }
                            }
                        }
                    },
                    abort_registration,
                );

                self.facts.spawn_detach(abrt);
                self.listeners_abort.push((abort_handle, listener_info));
                return Ok(local_addr);
            }
        }
    }

    fn server_conn<T: ServerTransport<F::IO>>(
        conn: T, facts: &F, server_close_rx: crossfire::MAsyncRx<()>,
    ) {
        let conn = Arc::new(conn);

        let dispatch = Arc::new(facts.new_dispatcher());
        let (done_tx, done_rx) = crossfire::mpsc::unbounded_async();

        let noti = RespNoti(done_tx);
        struct Reader<
            T: ServerTransport<F::IO>,
            F: ServerFacts,
            D: ReqDispatch<R>,
            R: ServerTaskResp,
        > {
            noti: RespNoti<R>,
            conn: Arc<T>,
            server_close_rx: crossfire::MAsyncRx<()>,
            dispatch: Arc<D>,
            logger: F::Logger,
        }

        impl<T: ServerTransport<F::IO>, F: ServerFacts, D: ReqDispatch<R>, R: ServerTaskResp>
            Reader<T, F, D, R>
        {
            async fn run(self) -> Result<(), ()> {
                loop {
                    match self.conn.read_req::<F>(&self.logger, &self.server_close_rx).await {
                        Ok(req) => {
                            if req.action == RpcAction::Num(0) && req.msg.len() == 0 {
                                // ping request
                                self.send_quick_resp(req.seq, None)?;
                            } else {
                                let seq = req.seq;
                                if self.dispatch.dispatch_req(req, self.noti.clone()).await.is_err()
                                {
                                    self.send_quick_resp(seq, Some(RpcIntErr::Decode.into()))?;
                                }
                            }
                        }
                        Err(_e) => {
                            // XXX read_req return error not used
                            return Err(());
                        }
                    }
                }
            }

            #[inline]
            fn send_quick_resp(&self, seq: u64, err: Option<RpcIntErr>) -> Result<(), ()> {
                if self.noti.send_err(seq, err).is_err() {
                    logger_warn!(self.logger, "{:?} reader abort due to writer has err", self.conn);
                    return Err(());
                }
                Ok(())
            }
        }
        let reader = Reader::<T, F, _, _> {
            noti,
            conn: conn.clone(),
            server_close_rx,
            dispatch: dispatch.clone(),
            logger: facts.new_logger(),
        };
        facts.spawn_detach(async move { reader.run().await });

        struct Writer<
            T: ServerTransport<F::IO>,
            F: ServerFacts,
            D: ReqDispatch<R>,
            R: ServerTaskResp,
        > {
            dispatch: Arc<D>,
            done_rx: crossfire::AsyncRx<Result<R, (u64, Option<RpcIntErr>)>>,
            conn: Arc<T>,
            logger: F::Logger,
        }

        impl<T: ServerTransport<F::IO>, F: ServerFacts, D: ReqDispatch<R>, R: ServerTaskResp>
            Writer<T, F, D, R>
        {
            async fn run(self) -> Result<(), io::Error> {
                macro_rules! process {
                    ($task: expr) => {{
                        match $task {
                            Ok(_task) => {
                                logger_trace!(self.logger, "write_resp {:?}", _task);
                                self.conn
                                    .write_resp::<F, R>(
                                        &self.logger,
                                        self.dispatch.get_codec(),
                                        _task,
                                    )
                                    .await?;
                            }
                            Err((seq, err)) => {
                                self.conn.write_resp_internal::<F>(&self.logger, seq, err).await?;
                            }
                        }
                    }};
                }
                while let Ok(task) = self.done_rx.recv().await {
                    process!(task);
                    while let Ok(task) = self.done_rx.try_recv() {
                        process!(task);
                    }
                    self.conn.flush_resp::<F>(&self.logger).await?;
                }
                logger_trace!(self.logger, "{:?} writer exits", self.conn);
                self.conn.close_conn::<F>(&self.logger).await;
                Ok(())
            }
        }
        let writer = Writer::<T, F, _, _> { done_rx, conn, dispatch, logger: facts.new_logger() };
        facts.spawn_detach(async move { writer.run().await });
    }

    #[inline]
    fn get_alive_conn(&self) -> usize {
        Arc::strong_count(&self.conn_ref_count) - 1
    }

    /// Gracefully close the server
    ///
    /// Steps:
    /// - listeners coroutine is abort
    /// - drop the close channel to notify connection read coroutines.
    /// - the writer coroutines will exit after all the reference of RespNoti channel drop to 0
    /// - wait for connection coroutines to exit with a timeout defined by
    /// ServerConfig.server_close_wait
    pub async fn close(&mut self) {
        // close listeners
        for h in &self.listeners_abort {
            h.0.abort();
            logger_info!(self.logger, "{} has closed", h.1);
        }
        // Notify all reader connection exit, then the reader will notify writer
        let _ = self.server_close_tx.lock().unwrap().take();

        let mut exists_count = self.get_alive_conn();
        // wait client close all connections
        let start_ts = Instant::now();
        let config = self.facts.get_config();
        while exists_count > 0 {
            <F::IO as AsyncIO>::sleep(Duration::from_secs(1)).await;
            exists_count = self.get_alive_conn();
            if Instant::now().duration_since(start_ts) > config.server_close_wait {
                logger_warn!(
                    self.logger,
                    "closed as wait too long for all conn closed voluntarily({} conn left)",
                    exists_count,
                );
                break;
            }
        }
        logger_info!(self.logger, "server closed with alive conn {}", exists_count);
    }
}

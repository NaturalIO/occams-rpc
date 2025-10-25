pub mod task;
pub use task::*;

#[cfg(feature = "tokio")]
pub type APIClientDefault<C> = occams_rpc_tokio::ClientDefault<APIClientReq, C>;
#[cfg(all(not(feature = "tokio"), feature = "smol"))]
pub type APIClientDefault<C> = occams_rpc_smol::ClientDefault<APIClientReq, C>;

pub use occams_rpc_api_macros::endpoint_async;
pub use occams_rpc_stream::client::ClientCaller;

use occams_rpc_core::Codec;
use occams_rpc_core::error::{EncodedErr, RpcErrCodec, RpcError, RpcIntErr};
pub use occams_rpc_stream::client::{
    ClientCallerBlocking, ClientFacts, ClientPool, ClientTransport, FailoverPool,
};
use std::fmt;
use std::sync::Arc;

pub type ClientDefault<IO, C> = occams_rpc_stream::client::ClientDefault<APIClientReq, IO, C>;

pub trait APIClientFacts: ClientFacts<Task = APIClientReq> {
    fn create_endpoint_async<T: ClientTransport>(
        self: Arc<Self>, addr: &str,
    ) -> AsyncEndpoint<ClientPool<Self, T>> {
        return AsyncEndpoint::new(ClientPool::new(self.clone(), addr, 0));
    }

    fn create_endpoint_async_failover<T: ClientTransport>(
        self: Arc<Self>, addrs: Vec<String>, round_robin: bool, retry_limit: usize,
    ) -> AsyncEndpoint<Arc<FailoverPool<Self, T>>> {
        return AsyncEndpoint::new(Arc::new(FailoverPool::new(
            self.clone(),
            addrs,
            round_robin,
            retry_limit,
            0,
        )));
    }

    fn create_endpoint_blocking<T: ClientTransport>(
        self: Arc<Self>, addr: &str,
    ) -> BlockingEndpoint<ClientPool<Self, T>> {
        return BlockingEndpoint::new(ClientPool::new(self.clone(), addr, 0));
    }

    fn create_endpoint_blocking_failover<T: ClientTransport>(
        self: Arc<Self>, addrs: Vec<String>, round_robin: bool, retry_limit: usize,
    ) -> BlockingEndpoint<Arc<FailoverPool<Self, T>>> {
        return BlockingEndpoint::new(Arc::new(FailoverPool::new(
            self.clone(),
            addrs,
            round_robin,
            retry_limit,
            0,
        )));
    }
}

impl<F: ClientFacts<Task = APIClientReq>> APIClientFacts for F {}

pub struct AsyncEndpoint<C>
where
    C: ClientCaller<Facts: ClientFacts<Task = APIClientReq>>,
{
    caller: C,
    codec: <C::Facts as ClientFacts>::Codec,
}

impl<C> AsyncEndpoint<C>
where
    C: ClientCaller<Facts: ClientFacts<Task = APIClientReq>>,
{
    pub fn new(caller: C) -> Self {
        Self { caller, codec: Default::default() }
    }

    pub async fn call<Req, Resp, E>(
        &self, service_method: &'static str, req: &Req,
    ) -> Result<Resp, RpcError<E>>
    where
        Req: serde::Serialize + fmt::Debug,
        Resp: for<'a> serde::Deserialize<'a> + Send + fmt::Debug + 'static + Default,
        E: RpcErrCodec,
    {
        let (tx, rx) = crossfire::spsc::bounded_tx_blocking_rx_async::<APIClientReq>(1);
        // TODO should optimize one shot channel
        <C as ClientCaller>::send_req(&self.caller, make_req(&self.codec, service_method, req, tx))
            .await;
        return process_res(&self.codec, rx.recv().await);
    }
}

impl<C> Clone for AsyncEndpoint<C>
where
    C: Clone + ClientCaller<Facts: ClientFacts<Task = APIClientReq>>,
{
    fn clone(&self) -> Self {
        Self::new(self.caller.clone())
    }
}

impl<C> std::ops::Deref for AsyncEndpoint<C>
where
    C: ClientCaller<Facts: ClientFacts<Task = APIClientReq>>,
{
    type Target = C;

    fn deref(&self) -> &Self::Target {
        &self.caller
    }
}

pub struct BlockingEndpoint<C>
where
    C: ClientCallerBlocking<Facts: ClientFacts<Task = APIClientReq>>,
{
    caller: C,
    codec: <C::Facts as ClientFacts>::Codec,
}

impl<C> BlockingEndpoint<C>
where
    C: ClientCallerBlocking<Facts: ClientFacts<Task = APIClientReq>>,
{
    fn new(caller: C) -> Self {
        Self { caller, codec: Default::default() }
    }

    pub fn call<Req, Resp, E>(
        &self, service_method: &'static str, req: &Req,
    ) -> Result<Resp, RpcError<E>>
    where
        Req: serde::Serialize + fmt::Debug,
        Resp: for<'a> serde::Deserialize<'a> + Send + fmt::Debug + 'static + Default,
        E: RpcErrCodec,
    {
        let (tx, rx) = crossfire::spsc::bounded_blocking::<APIClientReq>(1);
        // TODO should optimize one shot channel
        self.caller.send_req_blocking(make_req(&self.codec, service_method, req, tx));
        return process_res(&self.codec, rx.recv());
    }
}

impl<C> Clone for BlockingEndpoint<C>
where
    C: Clone + ClientCallerBlocking<Facts: ClientFacts<Task = APIClientReq>>,
{
    fn clone(&self) -> Self {
        Self::new(self.caller.clone())
    }
}

impl<C> std::ops::Deref for BlockingEndpoint<C>
where
    C: ClientCallerBlocking<Facts: ClientFacts<Task = APIClientReq>>,
{
    type Target = C;

    fn deref(&self) -> &Self::Target {
        &self.caller
    }
}

#[inline]
fn make_req<C, Req>(
    codec: &C, service_method: &'static str, req: &Req, done_tx: crossfire::Tx<APIClientReq>,
) -> APIClientReq
where
    C: Codec,
    Req: serde::Serialize + fmt::Debug,
{
    let req_buf = codec.encode(req).expect("encode");
    APIClientReq {
        common: Default::default(),
        req_msg: Some(req_buf),
        action: service_method.to_string(),
        resp: None,
        res: None,
        noti: Some(done_tx),
    }
}

#[inline]
fn process_res<C, Resp, E>(
    codec: &C, task_res: Result<APIClientReq, crossfire::RecvError>,
) -> Result<Resp, RpcError<E>>
where
    C: Codec,
    Resp: for<'a> serde::Deserialize<'a> + Send + fmt::Debug + 'static + Default,
    E: RpcErrCodec,
{
    match task_res {
        Ok(mut task) => {
            let res = task.res.take().unwrap();
            match res {
                Ok(()) => {
                    if let Some(resp) = task.resp {
                        match codec.decode(&resp) {
                            Ok(resp_msg) => return Ok(resp_msg),
                            Err(()) => return Err(RpcIntErr::Decode.into()),
                        }
                    } else {
                        return Ok(Resp::default());
                    }
                }
                Err(EncodedErr::Rpc(e)) => {
                    return Err(RpcError::Rpc(e));
                }
                Err(EncodedErr::Num(n)) => match E::decode(codec, Ok(n)) {
                    Ok(e) => return Err(RpcError::User(e)),
                    Err(()) => return Err(RpcIntErr::Decode.into()),
                },
                Err(EncodedErr::Buf(buf)) => match E::decode(codec, Err(&buf)) {
                    Ok(e) => return Err(RpcError::User(e)),
                    Err(()) => return Err(RpcIntErr::Decode.into()),
                },
                _ => unreachable!(),
            }
        }
        Err(_) => {
            return Err(RpcIntErr::Internal.into());
        }
    }
}

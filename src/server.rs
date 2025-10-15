use io_buffer::Buffer;
use occams_rpc_core::{Codec, error, error::ServerErr};
use occams_rpc_stream::server::{RespNoti, RespReceiver};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::sync::Arc;

pub struct Request<C: Codec> {
    pub seq: u64,
    pub service: String,
    pub method: String,
    pub req: Option<Vec<u8>>,
    pub codec: Arc<C>,
    pub noti: RespNoti<Response>,
}

impl<C: Codec> Request<C> {
    #[inline]
    pub fn decode<'a, R: Deserialize<'a>>(&'a mut self, buf: &'a [u8]) -> Result<R, ()> {
        self.codec.decode::<R>(buf)
    }

    #[inline(always)]
    pub fn set_result<R: Serialize>(self, resp: R) {
        match self.codec.encode::<R>(&resp) {
            Err(()) => {
                self.noti.done(Response {
                    seq: self.seq,
                    msg: None,
                    res: Err(error::RpcIntErr::Encode.into()),
                });
            }
            Ok(msg) => {
                self.noti.done(Response { seq: self.seq, msg: Some(msg), res: Ok(()) });
            }
        }
    }

    #[inline(always)]
    pub fn set_error(self, e: ServerErr) {
        self.noti.done(Response { seq: self.seq, msg: None, res: Err(e) });
    }
}

pub struct Response {
    pub seq: u64,
    pub msg: Option<Vec<u8>>,
    pub res: Result<(), ServerErr>,
}

impl fmt::Debug for Response {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "resp {} res {:?}", self.seq, self.res)
    }
}

/// RespReceiver for the API interface
pub struct RespReceiverAPI();

impl RespReceiver for RespReceiverAPI {
    type ChannelItem = Response;

    #[inline]
    fn encode_resp<'a, C: Codec>(
        _codec: &'a C, item: &'a mut Self::ChannelItem,
    ) -> (u64, Result<(Vec<u8>, Option<&'a Buffer>), &'a ServerErr>) {
        match &mut item.res {
            Ok(()) => {
                let msg = item.msg.take().unwrap();
                (item.seq, Ok((msg, None)))
            }
            Err(e) => (item.seq, Err(e)),
        }
    }
}

/// Generate code by macro
pub trait ServiceTrait<C: Codec>: Send + Sized + 'static {
    /// match req.method
    ///     match req.decode::<RequestType>() {
    ///         Err(())=>{
    ///             req.set_error(occams_rpc_core::error::RPC_ERR_DECODE);
    ///             returnl
    ///         }
    ///         Ok(arg)=>{
    ///             match self.#method(arg).await {
    ///                 Ok(resp)=>{
    ///                     req.set_result(resp);
    ///                 }
    ///                 Err(e)=>{
    ///                     req.set_error()
    ///                 }
    ///             }
    ///         }
    ///     }
    fn serve(&self, req: Request<C>) -> impl Future<Output = ()> + Send;
}

impl<S: ServiceTrait<C> + Sync + Send, C: Codec> ServiceTrait<C> for Arc<S> {
    #[inline]
    fn serve(&self, req: Request<C>) -> impl Future<Output = ()> + Send {
        self.as_ref().serve(req)
    }
}

use occams_rpc_core::{
    Codec,
    error::{EncodedErr, RpcErrCodec, RpcError, RpcIntErr},
};
use occams_rpc_stream::server::{RespNoti, ServerTaskEncode};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::io::Write;
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
                    res: Some(Err(RpcIntErr::Encode.into())),
                });
            }
            Ok(msg) => {
                self.noti.done(Response { seq: self.seq, msg: Some(msg), res: Some(Ok(())) });
            }
        }
    }

    #[inline(always)]
    pub fn set_rpc_error(self, e: RpcIntErr) {
        self.noti.done(Response { seq: self.seq, msg: None, res: Some(Err(e.into())) });
    }

    #[inline(always)]
    pub fn set_error<E: RpcErrCodec>(self, e: RpcError<E>) {
        let encoded_err = match e {
            RpcError::Rpc(rpc_int_err) => rpc_int_err.into(),
            RpcError::User(user_err) => user_err.encode(self.codec.as_ref()),
        };
        self.noti.done(Response { seq: self.seq, msg: None, res: Some(Err(encoded_err)) });
    }
}

pub struct Response {
    pub seq: u64,
    pub msg: Option<Vec<u8>>,
    pub res: Option<Result<(), EncodedErr>>,
}

impl fmt::Debug for Response {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "resp {} res {:?}", self.seq, self.res)
    }
}

impl ServerTaskEncode for Response {
    #[inline]
    fn encode_resp<'a, 'b, C: Codec>(
        &'a mut self, _codec: &'b C, buf: &'b mut Vec<u8>,
    ) -> (u64, Result<(usize, Option<&'a [u8]>), EncodedErr>) {
        match self.res.take().unwrap() {
            Ok(()) => {
                if let Some(msg) = self.msg.as_ref() {
                    buf.write_all(&msg).expect("fill msg buf");
                    return (self.seq, Ok((msg.len(), None)));
                } else {
                    return (self.seq, Ok((0, None)));
                }
            }
            Err(e) => {
                return (self.seq, Err(e));
            }
        }
    }
}

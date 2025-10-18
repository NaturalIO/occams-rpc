use crossfire::MTx;
use occams_rpc_core::Codec;
use occams_rpc_core::error::{EncodedErr, RpcIntErr};
use occams_rpc_stream::client::{
    ClientTask, ClientTaskAction, ClientTaskCommon, ClientTaskDecode, ClientTaskDone,
    ClientTaskEncode,
};
use occams_rpc_stream::proto::RpcAction;
use std::fmt;
use std::io::Write;

pub struct ClientReq {
    pub common: ClientTaskCommon,
    pub req_msg: Option<Vec<u8>>,
    /// action is in "Service.method" format
    pub action: String,
    pub resp: Option<Vec<u8>>,
    res: Option<Result<(), EncodedErr>>,
    noti: Option<MTx<Self>>,
}

impl ClientTaskEncode for ClientReq {
    #[inline]
    fn encode_req<C: Codec>(&self, _codec: &C, buf: &mut Vec<u8>) -> Result<usize, ()> {
        if let Some(msg) = self.req_msg.as_ref() {
            // The msg is pre encoded
            buf.write_all(msg).expect("append msg");
            Ok(msg.len())
        } else {
            Ok(0)
        }
    }
}

impl ClientTaskDecode for ClientReq {
    #[inline]
    fn decode_resp<C: Codec>(&mut self, _codec: &C, buf: &[u8]) -> Result<(), ()> {
        // Ignore the Codec, as we don't known the resp type yet
        if buf.len() > 0 {
            self.resp.replace(buf.to_vec());
        }
        Ok(())
    }
}

impl ClientTaskDone for ClientReq {
    #[inline]
    fn set_custom_error<C: Codec>(&mut self, _codec: &C, e: EncodedErr) {
        // Ignore the Codec, as we don't known the error type yet
        self.res.replace(Err(e.into()));
    }

    #[inline]
    fn set_rpc_error(&mut self, e: RpcIntErr) {
        self.res.replace(Err(e.into()));
    }

    #[inline]
    fn set_ok(&mut self) {
        self.res.replace(Ok(()));
    }

    #[inline]
    fn done(mut self) {
        let _ = self.noti.take().unwrap().send(self);
    }
}

impl std::ops::Deref for ClientReq {
    type Target = ClientTaskCommon;
    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.common
    }
}

impl std::ops::DerefMut for ClientReq {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.common
    }
}

impl ClientTaskAction for ClientReq {
    #[inline]
    fn get_action<'a>(&'a self) -> RpcAction<'a> {
        RpcAction::Str(self.action.as_str())
    }
}

impl fmt::Debug for ClientReq {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "ClientReq(seq={}, action={})", self.seq, self.action)
    }
}

impl ClientTask for ClientReq {}

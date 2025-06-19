use crate::error::*;
use io_engine::buffer::Buffer;

#[derive(Debug, Clone, Copy)]
pub enum RpcAction {
    Str(&'static str),
    Num(i32),
}

pub trait RpcTask: Sync + Sized + std::fmt::Display {
    fn seq(&self) -> u64;

    fn set_seq(&mut self, seq: u64);

    fn action(&self) -> RpcAction;

    #[inline(always)]
    /// This contain ext data only for request io operation, at most time it's empty
    fn get_ext_buf_req(&self) -> Option<&[u8]> {
        None
    }

    #[inline(always)]
    /// This contain preallocated ext data only for response io operation, at most time it's empty
    /// If buffer cannot be pre-allocated (when size varies) , should implement set_ext_buf()
    fn get_ext_buf_resp(&mut self, _blob_len: i32) -> Option<&mut [u8]> {
        None
    }

    fn get_msg_buf_req(&self) -> Option<Vec<u8>>;

    fn set_result(self, res: Result<&[u8], RPCError>);
}

pub struct RetryTaskInfo<T: RpcTask + Send + Unpin + 'static> {
    pub task: T,
    pub task_err: RPCError,
}

#[derive(Debug)]
pub struct RpcRequest {
    pub seq: u64,
    pub action: RpcAction,
    pub msg: Option<Vec<u8>>,
    pub blob: Option<Buffer>, // for write, this contains data
}

/// For Server-side
#[derive(Debug)]
pub struct RpcRespServer<'a> {
    pub seq: u64,
    /// On Ok((msg, blob))
    pub res: Result<(Option<Vec<u8>>, Option<&'a [u8]>), RPCError>,
}

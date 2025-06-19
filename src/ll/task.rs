use crate::error::*;
use io_engine::buffer::Buffer;

#[derive(Debug, Clone, Copy)]
pub enum RpcAction<'a> {
    Str(&'a str),
    Num(i32),
}

pub trait RpcClientTask: Sync + Sized + std::fmt::Display {
    fn seq(&self) -> u64;

    fn set_seq(&mut self, seq: u64);

    fn action<'a>(&'a self) -> RpcAction<'a>;

    #[inline(always)]
    /// Contain optional extra data to send to server side.
    fn get_req_ext_buf(&self) -> Option<&[u8]> {
        None
    }

    /// Get a mut buffer ref for large blob to hold optional extra data response from server
    ///
    /// `blob_len` is from RespHead, this interface should make sure that buffer of such size allocated.
    #[inline(always)]
    fn get_resp_ext_buf_mut(&mut self, _blob_len: i32) -> Option<&mut [u8]> {
        None
    }

    /// Return a sererialized msg of the request.
    fn get_req_msg_buf(&self) -> Option<Vec<u8>>;

    /// Set the result of response. The implementation should deserialize on Ok(buf)
    fn set_result(self, res: Result<&[u8], RpcError>);
}

pub struct RetryTaskInfo<T: RpcClientTask + Send + Unpin + 'static> {
    pub task: T,
    pub task_err: RpcError,
}

/// A temporary struct to hold data buffer return by RpcSrvReader::recv_req().
///
/// NOTE: you should consume the buffer ref before recv another request.
#[derive(Debug)]
pub struct RpcSvrReq<'a> {
    pub seq: u64,
    pub action: RpcAction<'a>,
    pub msg: Option<&'a [u8]>,
    pub blob: Option<Buffer>, // for write, this contains data
}

/// A struct hold response message
#[derive(Debug)]
pub struct RpcSvrResp {
    pub seq: u64,
    /// On Ok((msg, blob))
    pub res: Result<(Option<Vec<u8>>, Option<Buffer>), RpcError>,
}

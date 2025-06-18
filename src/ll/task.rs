use crate::error::*;
use io_engine::buffer::Buffer;

pub trait RpcTask: Sync + Sized + std::fmt::Display {
    fn seq(&self) -> u64;

    fn set_seq(&mut self, seq: u64);

    fn action(&self) -> u8;

    #[inline(always)]
    /// This contain ext data only for request io operation, at most time it's empty
    fn get_ext_buf_ref(&self) -> Option<&[u8]> {
        None
    }

    #[inline(always)]
    /// This contain preallocated ext data only for response io operation, at most time it's empty
    /// If buffer cannot be pre-allocated (when size varies) , should implement set_ext_buf()
    fn get_ext_buf_mut(&mut self, _blob_len: i32) -> Option<&mut [u8]> {
        None
    }

    fn get_msg_buf(&self) -> Option<Vec<u8>>;

    fn fill_task(&mut self, _msg: &[u8]) -> Result<(), RPCError> {
        // do nothing
        Ok(())
    }

    fn set_result(self, res: Result<(), RPCError>);
}

pub struct RetryTaskInfo<T: RpcTask + Send + Unpin + 'static> {
    pub task: T,
    pub task_err: RPCError,
}

#[derive(Default, Debug)]
pub struct RpcRequest {
    pub seq: u64,
    pub action: u8,
    pub msg: Option<Vec<u8>>,
    pub extension: Option<Buffer>, // for write, this contains data
}

#[derive(Default, Debug)]
pub struct RpcResponse<'a> {
    pub seq: u64,
    pub action: u8,
    pub err_no: u8,
    pub msg: Option<Vec<u8>>,
    pub extension: Option<&'a [u8]>, // for read, this contains data
}

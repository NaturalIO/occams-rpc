use super::task::{RpcAction, RpcClientTask, RpcSvrResp};
use crate::error::*;
use std::fmt;
use std::mem::{size_of, transmute};
use std::ptr::addr_of;
use zerocopy::{AsBytes, Unaligned};

pub const PING_ACTION: u32 = 0;

pub const RPC_MAGIC: [u8; 2] = [b'%', b'M'];
pub const U32_HIGH_MASK: u32 = 1 << 31;

pub const RESP_FLAG_HAS_ERRNO: u8 = 1;
pub const RESP_FLAG_HAS_ERR_STRING: u8 = 2;

/// Request:
///
/// Fixed len of ReqHead = 32B
/// | 2B   |1B | 1B    | 4B   | 8B  |  8B     |   4B  | 4B     |
/// | magic|ver| format|action| seq |client_id|msg_len|blob_len|
///
/// Variable length msg:
/// action_len
/// msg_len
/// blob_len
///
#[derive(AsBytes, Unaligned, PartialEq, Clone, Copy)]
#[repr(packed)]
pub struct ReqHead {
    pub magic: [u8; 2],
    pub ver: u8,
    pub format: u8,
    /// encoder-decoder format

    /// If highest bit is 0, the rest will be i32 action_num.
    ///
    /// If highest is 1, the lower bit will be i32 action_len.
    pub action: u32,

    /// Increased ID of request msg in the socket connection.
    pub seq: u64,

    pub client_id: u64,
    /// structured msg len
    pub msg_len: u32,
    /// unstructured msg
    pub blob_len: u32,
}

pub const RPC_REQ_HEADER_LEN: usize = size_of::<ReqHead>();

impl ReqHead {
    #[inline(always)]
    pub fn encode<'a, T>(
        client_id: u64, task: &'a T,
    ) -> (Self, Option<&'a [u8]>, Option<Vec<u8>>, Option<&'a [u8]>)
    where
        T: RpcClientTask,
    {
        let action_flag: u32;
        let mut action_str: Option<&'a [u8]> = None;
        match task.action() {
            RpcAction::Num(num) => action_flag = num as u32,
            RpcAction::Str(s) => {
                action_flag = s.len() as u32 | U32_HIGH_MASK;
                action_str = Some(s.as_bytes());
            }
        }
        let msg = task.get_req_msg_buf();
        let ext_buf = task.get_req_ext_buf();
        let msg_len = if let Some(msg_buf) = msg.as_ref() { msg_buf.len() as u32 } else { 0 };
        let blob_len = if let Some(blob) = ext_buf { blob.len() as u32 } else { 0 };
        // encode response header
        let header = ReqHead {
            magic: RPC_MAGIC,
            seq: task.seq(),
            client_id,
            ver: 1,
            format: 0,
            action: action_flag,
            msg_len,
            blob_len,
        };
        (header, action_str, msg, ext_buf)
    }

    #[inline(always)]
    pub fn decode(head_buf: &[u8]) -> Result<&Self, RpcError> {
        let _head: Option<&Self> = unsafe { transmute(head_buf.as_ptr()) };
        match _head {
            None => {
                return Err(RPC_ERR_COMM);
            }
            Some(head) => {
                if head.magic != RPC_MAGIC {
                    warn!("rpc server: wrong magic receive {:?}", head.magic);
                    return Err(RPC_ERR_COMM);
                }
                if head.ver != 1 {
                    warn!("rpc server: version {} not supported", head.ver);
                    return Err(RpcError::Rpc(ERR_NOT_SUPPORTED));
                }
                return Ok(head);
            }
        }
    }

    #[inline]
    pub fn get_action(&self) -> Result<i32, i32> {
        if self.action & U32_HIGH_MASK == 0 {
            Ok(self.action as i32)
        } else {
            let action_len = self.action ^ U32_HIGH_MASK;
            Err(action_len as i32)
        }
    }
}

impl fmt::Display for ReqHead {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let _ = unsafe {
            write!(
                f,
                "[client_id:{}, seq:{}, msg:{}, blob:{}",
                addr_of!(self.client_id).read_unaligned(),
                addr_of!(self.seq).read_unaligned(),
                addr_of!(self.msg_len).read_unaligned(),
                addr_of!(self.blob_len).read_unaligned(),
            )
        };
        match self.get_action() {
            Ok(action_num) => {
                write!(f, ", action:{:?}]", action_num)
            }
            Err(action_len) => {
                write!(f, "action_len:{}]", action_len)
            }
        }
    }
}

impl fmt::Debug for ReqHead {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl Default for ReqHead {
    fn default() -> Self {
        Self {
            magic: RPC_MAGIC,
            ver: 0,
            format: 0,
            action: 0,
            seq: 0,
            client_id: 0,
            msg_len: 0,
            blob_len: 0,
        }
    }
}

/// Response:
///
/// Fixed len of RespHead = 20B
/// | 2B   |1B | 1B      |  8B  |     4B  | 4B     |
/// | magic|ver| has_err |  seq | msg_len |blob_len|
///
/// Variable length msg:
/// msg_len
/// blob_len
///
#[derive(AsBytes, Unaligned, PartialEq, Clone, Copy)]
#[repr(packed)]
pub struct RespHead {
    pub magic: [u8; 2],
    pub ver: u8,

    /// when flag == RESP_FLAG_HAS_ERRNO: msg_len is posix errno; blob_len = 0
    /// when flag == RESP_FLAG_HAS_ERR_STRING: msg_len=0, blob_len > 0 and follow an error string
    pub flag: u8,

    /// structured msg_len or errno
    pub msg_len: u32,

    /// Increased ID of request msg in the socket connection (response.seq==request.seq)
    pub seq: u64,
    /// unstructured msg
    pub blob_len: u32,
}

pub const RPC_RESP_HEADER_LEN: usize = size_of::<RespHead>();

impl RespHead {
    pub fn encode<'a>(task_resp: &'a RpcSvrResp) -> (Self, Option<&'a Vec<u8>>, Option<&'a [u8]>) {
        let error_str: &[u8];
        let seq = task_resp.seq;
        match task_resp.res {
            Ok((ref msg, ref blob)) => {
                let msg_len = if let Some(msg_buf) = msg.as_ref() { msg_buf.len() } else { 0 };
                let mut blob_len: u32 = 0;
                let mut blob_o: Option<&[u8]> = None;
                if let Some(blob_buf) = blob.as_ref() {
                    blob_len = blob_buf.len() as u32;
                    blob_o = Some(blob_buf.as_ref());
                }
                let header = RespHead {
                    magic: RPC_MAGIC,
                    ver: 1,
                    flag: 0,
                    seq: task_resp.seq,
                    msg_len: msg_len as u32,
                    blob_len: blob_len as u32,
                };
                return (header, msg.as_ref(), blob_o);
            }
            Err(RpcError::Posix(errno)) => {
                let header = RespHead {
                    magic: RPC_MAGIC,
                    ver: 1,
                    flag: RESP_FLAG_HAS_ERRNO,
                    seq: task_resp.seq,
                    msg_len: errno as u32,
                    blob_len: 0,
                };
                return (header, None, None);
            }
            Err(RpcError::Rpc(s)) => {
                error_str = s.as_bytes();
            }
            Err(RpcError::Remote(ref s)) => {
                error_str = s.as_bytes();
            }
        }
        let header = RespHead {
            magic: RPC_MAGIC,
            ver: 1,
            flag: RESP_FLAG_HAS_ERR_STRING,
            seq,
            msg_len: 0,
            blob_len: error_str.len() as u32,
        };
        return (header, None, Some(error_str));
    }

    #[inline(always)]
    pub fn decode(head_buf: &[u8]) -> Result<&Self, RpcError> {
        let _head: Option<&Self> = unsafe { transmute(head_buf.as_ptr()) };
        match _head {
            None => {
                return Err(RPC_ERR_COMM);
            }
            Some(head) => {
                if head.magic != RPC_MAGIC {
                    warn!("rpc server: wrong magic receive {:?}", head.magic);
                    return Err(RPC_ERR_COMM);
                }
                if head.ver != 1 {
                    warn!("rpc server: version {} not supported", head.ver);
                    return Err(RpcError::Rpc(ERR_NOT_SUPPORTED));
                }
                return Ok(head);
            }
        }
    }
}

impl fmt::Display for RespHead {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        unsafe {
            write!(
                f,
                "[seq:{}, flag:{}, msg:{}, blob:{}]",
                addr_of!(self.seq).read_unaligned(), // format_args deals with unaligned field
                addr_of!(self.flag).read_unaligned(),
                addr_of!(self.msg_len).read_unaligned(),
                addr_of!(self.blob_len).read_unaligned(),
            )
        }
    }
}

impl fmt::Debug for RespHead {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl Default for RespHead {
    fn default() -> Self {
        Self { magic: RPC_MAGIC, ver: 0, flag: 0, msg_len: 0, seq: 0, blob_len: 0 }
    }
}

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn test_header_len() {
        assert_eq!(RPC_REQ_HEADER_LEN, 32);
        assert_eq!(RPC_RESP_HEADER_LEN, 20);
    }
}

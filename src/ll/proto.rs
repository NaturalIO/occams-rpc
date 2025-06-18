use crate::error::*;
use std::fmt;
use std::mem::{size_of, transmute};
use std::ptr::addr_of;
use zerocopy::{AsBytes, FromBytes, Unaligned};

pub const PING_ACTION: u32 = 0;

pub const RPC_MAGIC: [u8; 2] = [b'%', b'M'];
pub const U32_HIGH_MASK: u32 = 1 << 31;

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
    pub fn decode(head_buf: &[u8]) -> Result<&Self, RPCError> {
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
                    return Err(RPCError::Rpc(ERR_NOT_SUPPORTED));
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
        unsafe {
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
                write!(f, ", action:{}]", action_num,)
            }
            Err(action_len) => {
                write!(f, "action_len:{}]", action_len,)
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
/// Fixed len of RespHead = 28B
/// | 2B   |1B | 1B    | 4B  | 8B  | 4B     |   4B  | 4B       |
/// | magic|ver| format| err | seq | action | msg_len |blob_len|
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
    pub format: u8,

    ///If highest is 0, hold unix errno in i32.
    /// If highest is 1, the rest bit is set zero. msg_len is the error string.
    pub err: u32,

    /// Increased ID of request msg in the socket connection (response.seq==request.seq)
    pub seq: u64,

    /// If highest bit is 0, the rest will be i32 action_num.
    ///
    /// If highest is 1, the lower bit will be no meaning,
    pub action: u32,

    /// structured msg len
    pub msg_len: u32,

    /// unstructured msg
    pub blob_len: u32,
}

pub const RPC_RESP_HEADER_LEN: usize = size_of::<RespHead>();

impl RespHead {
    #[inline(always)]
    pub fn decode(head_buf: &[u8]) -> Result<&Self, RPCError> {
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
                    return Err(RPCError::Rpc(ERR_NOT_SUPPORTED));
                }
                return Ok(head);
            }
        }
    }

    #[inline]
    pub fn get_error(&self) -> Result<i32, i32> {
        if self.err & U32_HIGH_MASK == 0 {
            Ok(self.err as i32)
        } else {
            let err_len = self.err ^ U32_HIGH_MASK;
            Err(err_len as i32)
        }
    }

    #[inline]
    pub fn get_action(&self) -> Result<i32, ()> {
        if self.action & U32_HIGH_MASK == 0 { Ok(self.action as i32) } else { Err(()) }
    }
}

impl fmt::Display for RespHead {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        unsafe {
            write!(
                f,
                "[seq:{}, format:{}, msg:{}, blob:{}",
                addr_of!(self.seq).read_unaligned(), // format_args deals with unaligned field
                addr_of!(self.format).read_unaligned(),
                addr_of!(self.msg_len).read_unaligned(),
                addr_of!(self.blob_len).read_unaligned(),
            )
        };
        match self.get_error() {
            Ok(err_no) => {
                write!(f, ", errno:{}]", err_no)
            }
            Err(err_len) => unsafe { write!(f, "err_len:{}]", err_len,) },
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
        Self {
            ver: 0,
            seq: 0,
            format: 0,
            action: 0,
            err_no: 0,
            msg_len: 0,
            blob_len: 0,
            magic: RPC_MAGIC,
        }
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn test_header_len() {
        assert_eq!(RPC_REQ_HEADER_LEN, 32);
        assert_eq!(RPC_REQ_HEADER_LEN, 28);
    }
}

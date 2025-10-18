//! # The protocol
//!
//! ## Request
//!
//! Fixed length of `ReqHead` = 32B
//!
//! | Field     | Size | Description                               |
//! |-----------|------|-------------------------------------------|
//! | `magic`   | 2B   | Magic number                              |
//! | `ver`     | 1B   | Protocol version                          |
//! | `format`  | 1B   | Encoder-decoder format                    |
//! | `action`  | 4B   | Action type (numeric or length if string) |
//! | `seq`     | 8B   | Increased ID of request message           |
//! | `client_id`| 8B   | Client identifier                         |
//! | `msg_len` | 4B   | Structured message length                 |
//! | `blob_len`| 4B   | Unstructured message (blob) length        |
//!
//! Variable length message components:
//! - `action_len` (if `action` is a string)
//! - `msg_len`
//! - `blob_len`
//!
//! ## Response:
//!
//! Fixed length of `RespHead` = 20B
//!
//! | Field     | Size | Description                               |
//! |-----------|------|-------------------------------------------|
//! | `magic`   | 2B   | Magic number                              |
//! | `ver`     | 1B   | Protocol version                          |
//! | `has_err` | 1B   | Error flag                                |
//! | `seq`     | 8B   | Increased ID of request message           |
//! | `msg_len` | 4B   | Structured message length or errno        |
//! | `blob_len`| 4B   | Unstructured message (blob) length        |
//!
//! Variable length message components:
//! - `msg_len`
//! - `blob_len`
///
use crate::client::ClientTask;
use io_buffer::Buffer;
use occams_rpc_core::{Codec, error::*};
use std::fmt;
use std::io::Write;
use std::mem::size_of;
use std::ptr::addr_of;
use zerocopy::byteorder::little_endian;
use zerocopy::{FromBytes, Immutable, IntoBytes, KnownLayout, Unaligned};

pub const PING_ACTION: u32 = 0;

pub const RPC_MAGIC: little_endian::U16 = little_endian::U16::new(19749);
pub const U32_HIGH_MASK: u32 = 1 << 31;

pub const RESP_FLAG_HAS_ERRNO: u8 = 1;
pub const RESP_FLAG_HAS_ERR_STRING: u8 = 2;
pub const RPC_VERSION_1: u8 = 1;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RpcAction<'a> {
    Str(&'a str),
    Num(i32),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RpcActionOwned {
    Str(String),
    Num(i32),
}

impl<'a> From<RpcAction<'a>> for RpcActionOwned {
    fn from(action: RpcAction<'a>) -> Self {
        match action {
            RpcAction::Str(s) => RpcActionOwned::Str(s.to_string()),
            RpcAction::Num(n) => RpcActionOwned::Num(n),
        }
    }
}

impl RpcActionOwned {
    pub fn to_action<'a>(&'a self) -> RpcAction<'a> {
        match self {
            RpcActionOwned::Str(s) => RpcAction::Str(s.as_str()),
            RpcActionOwned::Num(n) => RpcAction::Num(*n),
        }
    }
}

/// Fixed-length header for request
#[derive(FromBytes, IntoBytes, Unaligned, Immutable, KnownLayout, PartialEq, Clone)]
#[repr(packed)]
pub struct ReqHead {
    pub magic: little_endian::U16,
    pub ver: u8,
    pub format: u8,
    /// encoder-decoder format

    /// If highest bit is 0, the rest will be i32 action_num.
    ///
    /// If highest is 1, the lower bit will be i32 action_len.
    pub action: little_endian::U32,

    /// Increased ID of request msg in the socket connection.
    pub seq: little_endian::U64,

    pub client_id: little_endian::U64,
    /// structured msg len
    pub msg_len: little_endian::U32,
    /// unstructured msg
    pub blob_len: little_endian::U32,
}

pub const RPC_REQ_HEADER_LEN: usize = size_of::<ReqHead>();

impl ReqHead {
    #[inline(always)]
    pub fn encode_ping(buf: &mut Vec<u8>, client_id: u64, seq: u64) {
        debug_assert!(buf.capacity() > RPC_REQ_HEADER_LEN);
        unsafe { buf.set_len(RPC_REQ_HEADER_LEN) };
        let header: &mut Self = Self::mut_from_bytes(buf).expect("fill header buf");
        header.magic = RPC_MAGIC;
        header.ver = RPC_VERSION_1;
        header.format = 0;
        header.action.set(PING_ACTION);
        header.seq.set(seq);
        header.client_id.set(client_id);
        header.msg_len.set(0);
        header.blob_len.set(0);
    }

    /// write header, action, msg into `buf`, return reference to blob if there's any
    #[inline(always)]
    pub fn encode<'a, T, C>(
        codec: &C, buf: &mut Vec<u8>, client_id: u64, task: &'a T,
    ) -> Result<Option<&'a [u8]>, ()>
    where
        T: ClientTask,
        C: Codec,
    {
        debug_assert!(buf.capacity() >= RPC_REQ_HEADER_LEN);
        // Leave a room at the begining of buffer for ReqHead
        unsafe { buf.set_len(RPC_REQ_HEADER_LEN) };
        // But we have to write action str and encode the msg first to get message data len
        let action_flag: u32;
        match task.get_action() {
            RpcAction::Num(num) => action_flag = num as u32,
            RpcAction::Str(s) => {
                action_flag = s.len() as u32 | U32_HIGH_MASK;
                buf.write_all(s.as_bytes()).expect("fill action buffer");
            }
        }
        let msg_len = task.encode_req(codec, buf)?;
        let blob = task.get_req_blob();
        let blob_len = if let Some(blob) = blob { blob.len() as u32 } else { 0 };
        let header: &mut Self =
            Self::mut_from_bytes(&mut buf[..RPC_REQ_HEADER_LEN]).expect("fill header buf");
        // NOTE: We are directly init ReqHead on the buffer with unsafe, check carefully don't miss
        // a field
        header.magic = RPC_MAGIC;
        header.ver = RPC_VERSION_1;
        header.format = 0;
        header.action.set(action_flag);
        header.seq.set(task.seq());
        header.client_id.set(client_id);
        header.msg_len.set(msg_len as u32);
        header.blob_len.set(blob_len);
        Ok(blob)
    }

    #[inline(always)]
    pub fn decode_head(head_buf: &[u8]) -> Result<&Self, RpcIntErr> {
        let head: &Self = Self::ref_from_bytes(head_buf).expect("from header buf");
        if head.magic != RPC_MAGIC {
            warn!("rpc server: wrong magic receive {:?}", head.magic);
            return Err(RpcIntErr::IO);
        }
        if head.ver != RPC_VERSION_1 {
            warn!("rpc server: version {} not supported", head.ver);
            return Err(RpcIntErr::Version);
        }
        return Ok(head);
    }

    #[inline]
    pub fn get_action(&self) -> Result<i32, i32> {
        if self.action & U32_HIGH_MASK == 0 {
            Ok(self.action.get() as i32)
        } else {
            let action_len = self.action ^ U32_HIGH_MASK;
            Err(action_len.get() as i32)
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

/// Fixed-length header for response
#[derive(FromBytes, IntoBytes, Unaligned, Immutable, KnownLayout, PartialEq, Clone)]
#[repr(packed)]
pub struct RespHead {
    pub magic: little_endian::U16,
    pub ver: u8,

    /// when flag == RESP_FLAG_HAS_ERRNO: msg_len is posix errno; blob_len = 0
    /// when flag == RESP_FLAG_HAS_ERR_STRING: msg_len=0, blob_len > 0 and follow an error string
    pub flag: u8,

    /// structured msg_len or errno
    pub msg_len: little_endian::U32,

    /// Increased ID of request msg in the socket connection (response.seq==request.seq)
    pub seq: little_endian::U64,
    /// unstructured msg, only support half of 16Byte, must larger than zero
    pub blob_len: little_endian::I32,
}

pub const RPC_RESP_HEADER_LEN: usize = size_of::<RespHead>();

impl RespHead {
    #[inline]
    pub fn encode_err<'a>(seq: u64, err: &'a EncodedErr) -> (Self, Option<&'a [u8]>) {
        let error_str: &[u8];
        match err {
            EncodedErr::Num(n) => {
                let header = RespHead {
                    magic: RPC_MAGIC,
                    ver: RPC_VERSION_1,
                    flag: RESP_FLAG_HAS_ERRNO,
                    seq: little_endian::U64::new(seq),
                    msg_len: little_endian::U32::new(*n as u32),
                    blob_len: little_endian::I32::ZERO,
                };
                return (header, None);
            }
            EncodedErr::Rpc(s) => {
                error_str = s.as_bytes();
            }
            EncodedErr::Buf(s) => {
                error_str = &s;
            }
            EncodedErr::Static(s) => {
                error_str = s.as_bytes();
            }
        }
        let header = RespHead {
            magic: RPC_MAGIC,
            ver: RPC_VERSION_1,
            flag: RESP_FLAG_HAS_ERR_STRING,
            seq: little_endian::U64::new(seq),
            msg_len: little_endian::U32::ZERO,
            blob_len: little_endian::I32::new(error_str.len() as i32),
        };
        return (header, Some(error_str));
    }

    #[inline]
    pub fn encode_msg(seq: u64, msg: &[u8], blob: Option<&Buffer>) -> Self {
        let mut blob_len: i32 = 0;
        if let Some(blob_buf) = blob.as_ref() {
            blob_len = blob_buf.len() as i32;
        }
        let header = RespHead {
            magic: RPC_MAGIC,
            ver: RPC_VERSION_1,
            flag: 0,
            seq: little_endian::U64::new(seq),
            msg_len: little_endian::U32::new(msg.len() as u32),
            blob_len: little_endian::I32::new(blob_len as i32),
        };
        return header;
    }

    #[inline(always)]
    pub fn as_bytes(&self) -> &[u8] {
        zerocopy::IntoBytes::as_bytes(self)
    }

    #[inline(always)]
    pub fn decode_head(head_buf: &[u8]) -> Result<&Self, RpcIntErr> {
        let head: &Self = Self::ref_from_bytes(head_buf).expect("decode header");
        if head.magic != RPC_MAGIC {
            warn!("rpc server: wrong magic receive {:?}", head.magic);
            return Err(RpcIntErr::IO);
        }
        if head.ver != RPC_VERSION_1 {
            warn!("rpc server: version {} not supported", head.ver);
            return Err(RpcIntErr::Version);
        }
        return Ok(head);
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

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn test_header_len() {
        assert_eq!(RPC_REQ_HEADER_LEN, 32);
        assert_eq!(RPC_RESP_HEADER_LEN, 20);
    }
}

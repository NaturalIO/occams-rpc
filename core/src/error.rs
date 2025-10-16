use crate::Codec;
use serde::{Deserialize, Serialize};
use std::convert::{TryFrom, TryInto};
use std::fmt;

pub const RPC_ERR_PREFIX: &'static str = "rpc_";

/// A error type defined by client-side user logic
///
/// Due to possible decode
#[derive(thiserror::Error)]
pub enum RpcError<E: RpcErrCodec> {
    User(E),
    Rpc(RpcIntErr),
}

impl<E: RpcErrCodec> std::cmp::PartialEq<RpcIntErr> for RpcError<E> {
    fn eq(&self, other: &RpcIntErr) -> bool {
        if let Self::Rpc(r) = self {
            if r == other {
                return true;
            }
        }
        false
    }
}

impl<E: RpcErrCodec + PartialEq> std::cmp::PartialEq<RpcError<E>> for RpcError<E> {
    fn eq(&self, other: &Self) -> bool {
        match self {
            Self::Rpc(r) => {
                if let Self::Rpc(o) = other {
                    return r == o;
                }
            }
            Self::User(r) => {
                if let Self::User(o) = other {
                    return r == o;
                }
            }
        }
        false
    }
}

//impl<E: RpcErrCodec> RpcError<E> {
//
//    #[inline]
//    fn encode<C: Codec>(&self, codec: &C) -> EncodedErr {
//        match self {
//            Self::User(e)=>{
//                e.encode(codec)
//            }
//            Self::Rpc(e)=>{
//                EncodedErr::Rpc(*e)
//            }
//        }
//    }
//}

impl<E: RpcErrCodec> From<RpcIntErr> for RpcError<E> {
    #[inline]
    fn from(e: RpcIntErr) -> Self {
        Self::Rpc(e)
    }
}

/// Because Rust does not allow overlapping impl, we only imple RpcError trait for i32 and &str
/// and String. If you use other type as error, you should add and implementation with code
/// manually.
///
/// # Example
///
/// ```rust
/// use serde_derive::{Serialize, Deserialize};
/// use occams_rpc_core::{error::{RpcErrCodec, RpcIntErr}, Codec};
/// #[derive(Serialize, Deserialize)]
/// pub enum MyError {
///     NoSuchFile,
///     TooManyRequest,
/// }
///
/// impl RpcErrCodec for MyError {
///     #[inline(always)]
///     fn encode<C: Codec>(&self, codec: &C) -> EncodedErr {
///         match codec.encode(self) {
///             Ok(buf)=>EncodedErr::Buf(buf),
///             Err(())=>EncodedErr::Rpc(RpcIntErr::Encode),
///         }
///     }
///     #[inline(always)]
///     fn decode<C: Codec>(codec: &C, buf: Result<u32, &[u8]>) -> Result<Self, ()> {
///         if let Err(b) = buf {
///             return codec.decode(b);
///         } else {
///             Err(())
///         }
///     }
/// }
/// ```
pub trait RpcErrCodec: Send + Sized + 'static + Unpin {
    fn encode<C: Codec>(&self, codec: &C) -> EncodedErr;

    fn decode<C: Codec>(codec: &C, buf: Result<u32, &[u8]>) -> Result<Self, ()>;
}

macro_rules! impl_rpc_error_for_num {
    ($t: tt) => {
        impl RpcErrCodec for $t {
            #[inline(always)]
            fn encode<C: Codec>(&self, _codec: &C) -> EncodedErr {
                EncodedErr::Num(*self as i32)
            }

            #[inline(always)]
            fn decode<C: Codec>(_codec: &C, buf: Result<u32, &[u8]>) -> Result<Self, ()> {
                if let Ok(i) = buf {
                    if i <= $t::max as u32 {
                        return Ok(i as Self);
                    }
                }
                Err(())
            }
        }
    };
}

impl_rpc_error_for_num!(i8);
impl_rpc_error_for_num!(u8);
impl_rpc_error_for_num!(i16);
impl_rpc_error_for_num!(u16);
impl_rpc_error_for_num!(i32);
impl_rpc_error_for_num!(u32);

impl RpcErrCodec for nix::errno::Errno {
    #[inline(always)]
    fn encode<C: Codec>(&self, _codec: &C) -> EncodedErr {
        EncodedErr::Num(*self as i32)
    }

    #[inline(always)]
    fn decode<C: Codec>(_codec: &C, buf: Result<u32, &[u8]>) -> Result<Self, ()> {
        if let Ok(i) = buf {
            if i <= i32::max as u32 {
                return Ok(Self::from_i32(i as i32));
            }
        }
        Err(())
    }
}

impl RpcErrCodec for String {
    #[inline(always)]
    fn encode<C: Codec>(&self, _codec: &C) -> EncodedErr {
        EncodedErr::Buf(Vec::from(self.as_bytes()))
    }
    #[inline(always)]
    fn decode<C: Codec>(codec: &C, buf: Result<u32, &[u8]>) -> Result<Self, ()> {
        if let Err(s) = buf {
            if let Ok(s) = str::from_utf8(s) {
                return Ok(s.to_string());
            }
        }
        Err(())
    }
}

/// "rpc_" prefix is reserved for internal error
#[derive(
    strum::Display, strum::EnumString, strum::AsRefStr, Debug, PartialEq, Clone, thiserror::Error,
)]
#[repr(u8)]
pub enum RpcIntErr {
    /// Ping or connect error
    #[strum(serialize = "rpc_unreachable")]
    Unreachable,
    /// IO error
    #[strum(serialize = "rpc_io_err")]
    IO,
    /// Task timeout
    #[strum(serialize = "rpc_timeout")]
    Timeout,
    /// Method not found
    #[strum(serialize = "rpc_method_notfound")]
    Method,
    /// service notfound
    #[strum(serialize = "rpc_service_notfound")]
    Service,
    /// Encode Error
    #[strum(serialize = "rpc_encode")]
    Encode,
    /// Decode Error
    #[strum(serialize = "rpc_decode")]
    Decode,
    /// Internal error
    #[strum(serialize = "rpc_internal_err")]
    Internal,
    /// invalid version number in rpc header
    #[strum(serialize = "rpc_invalid_ver")]
    Version,
}

impl RpcIntErr {
    #[inline]
    pub fn as_bytes<'a>(&'a self) -> &'a [u8] {
        self.as_ref().as_bytes()
    }
}

/// A container for error message parse from / send into transport
#[derive(Debug, thiserror::Error)]
pub enum EncodedErr {
    /// The ClientTransport should try the best to parse it from string with "rpc_" prefix
    Rpc(RpcIntErr),
    /// For nix errno and the like
    Num(i32),
    /// only for server, the ClientTransport will not parse into static type
    Static(&'static str),
    /// The ClientTransport will fallback to Vec<u8> after try to parse  RpcIntErr and  num
    Buf(Vec<u8>),
}
//
//impl EncodedErr {
//    #[inline]
//    pub fn try_as_str<'a>(&'a self) -> Result<&'a str, ()> {
//        match self {
//            Self::Str(s) => return Ok(s),
//            Self::Text(s) => return Ok(s.as_str()),
//            Self::Buf(b) => {
//                if let Ok(s) = str::from_utf8(b) {
//                    return Ok(s);
//                }
//            }
//            _ => {}
//        }
//        Err(())
//    }
//}
//
//impl std::cmp::PartialEq<RpcIntErr> for EncodedErr {
//    fn eq(&self, other: &RpcIntErr) -> bool {
//        if let Self::Rpc(r) = self {
//            if r == other {
//                return true;
//            }
//        }
//        false
//    }
//}
//
//impl std::cmp::PartialEq<EncodedErr> for EncodedErr {
//    fn eq(&self, other: &EncodedErr) -> bool {
//        match self {
//            Self::Rpc(e) => {
//                if let Self::Rpc(o) = other {
//                    return e == o;
//                }
//            }
//            Self::Num(e) => {
//                if let Self::Num(o) = other {
//                    return e == o;
//                }
//            }
//            Self::Str(s) => {
//                if let Ok(o) = other.try_as_str() {
//                    return *s == o;
//                }
//            }
//            Self::Text(s) => {
//                if let Ok(o) = other.try_as_str() {
//                    return s == o;
//                }
//            }
//            Self::Buf(s) => {
//                if let Self::Buf(o) = other {
//                    return s == o;
//                } else if let Ok(o) = other.try_as_str() {
//                    // other's type is not Buf
//                    if let Ok(_s) = str::from_utf8(s) {
//                        return _s == o;
//                    }
//                }
//            }
//        }
//        false
//    }
//}
//
impl fmt::Display for EncodedErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Rpc(e) => e.fmt(f),
            Self::Num(no) => write!(f, "errno {}", no),
            Self::Static(s) => write!(f, "{}", s),
            Self::Buf(b) => match str::from_utf8(b) {
                Ok(s) => {
                    write!(f, "{}", s)
                }
                Err(_) => {
                    write!(f, "err blob {} length", b.len())
                }
            },
        }
    }
}

///// the same with errno
//impl From<i32> for EncodedErr {
//    #[inline(always)]
//    fn from(e: i32) -> Self {
//        EncodedErr::Num(e)
//    }
//}
//
//impl From<&'static str> for EncodedErr {
//    #[inline(always)]
//    fn from(s: &'static str) -> Self {
//        EncodedErr::Str(s)
//    }
//}
//
//impl From<&String> for EncodedErr {
//    #[inline(always)]
//    fn from(s: &String) -> Self {
//        EncodedErr::Text(s.to_string())
//    }
//}
//
//impl From<String> for EncodedErr {
//    #[inline(always)]
//    fn from(s: String) -> Self {
//        EncodedErr::Text(s)
//    }
//}
//
///// For nix::errno
//impl Into<i32> for EncodedErr {
//    #[inline(always)]
//    fn into(self) -> i32 {
//        if let EncodedErr::Num(i) = self {
//            return i as i32;
//        } else {
//            unimplemented!();
//        }
//    }
//}

impl From<RpcIntErr> for EncodedErr {
    #[inline(always)]
    fn from(e: RpcIntErr) -> Self {
        Self::Rpc(e)
    }
}
//
//#[cfg(test)]
//mod tests {
//    use super::*;
//    use nix::errno::Errno;
//    use std::str::FromStr;
//
//    #[test]
//    fn test_error_conversion() {
//        println!("{}", RpcIntErr::Internal);
//        println!("{:?}", RpcIntErr::Internal);
//        let s = RpcIntErr::Timeout.as_ref();
//        println!("RpcIntErr::Timeout as {}", s);
//        let e = RpcIntErr::from_str(s).expect("parse");
//        assert_eq!(e, RpcIntErr::Timeout);
//        assert!(RpcIntErr::from_str("timeoutss").is_err());
//
//        println!("test errno {:?}", Errno::EIO);
//        let e: EncodedErr = Errno::EIO.into();
//        println!("{}", e);
//        println!("{:?}", e);
//
//        println!("test from str");
//        let e = EncodedErr::from("err_str");
//        println!("{:?}", e);
//
//        let e = EncodedErr::from("err_str".to_string());
//        println!("{:?}", e);
//    }
//}

use crate::Codec;
use std::fmt;

pub const RPC_ERR_PREFIX: &'static str = "rpc_";

/// A error type defined by client-side user logic
///
/// Due to possible decode
#[derive(thiserror::Error, Debug)]
pub enum RpcError<E: RpcErrCodec> {
    User(E),
    Rpc(RpcIntErr),
}
impl<E: RpcErrCodec> fmt::Display for RpcError<E> {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::User(e) => fmt::Display::fmt(e, f),
            Self::Rpc(e) => fmt::Display::fmt(e, f),
        }
    }
}

impl<E: RpcErrCodec> std::cmp::PartialEq<RpcIntErr> for RpcError<E> {
    #[inline]
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
    #[inline]
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

impl<E: RpcErrCodec> From<E> for RpcError<E> {
    #[inline]
    fn from(e: E) -> Self {
        Self::User(e)
    }
}

impl From<&str> for RpcError<String> {
    #[inline]
    fn from(e: &str) -> Self {
        Self::User(e.to_string())
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
/// use occams_rpc_core::{error::{RpcErrCodec, RpcIntErr, EncodedErr}, Codec};
/// use strum::Display;
/// #[derive(Serialize, Deserialize, Debug, Display)]
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
pub trait RpcErrCodec: Send + Sized + 'static + Unpin + fmt::Debug + fmt::Display {
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
                return Ok(Self::from_raw(i as i32));
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
    fn decode<C: Codec>(_codec: &C, buf: Result<u32, &[u8]>) -> Result<Self, ()> {
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
    strum::Display, strum::EnumString, strum::AsRefStr, PartialEq, Clone, thiserror::Error,
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

// The default Debug derive just ignore strum customized string, by strum only have a Display derive
impl fmt::Debug for RpcIntErr {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl RpcIntErr {
    #[inline]
    pub fn as_bytes<'a>(&'a self) -> &'a [u8] {
        self.as_ref().as_bytes()
    }
}

impl From<std::io::Error> for RpcIntErr {
    #[inline(always)]
    fn from(_e: std::io::Error) -> Self {
        Self::IO
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

impl EncodedErr {
    #[inline]
    pub fn try_as_str<'a>(&'a self) -> Result<&'a str, ()> {
        match self {
            Self::Static(s) => return Ok(s),
            Self::Buf(b) => {
                if let Ok(s) = str::from_utf8(b) {
                    return Ok(s);
                }
            }
            _ => {}
        }
        Err(())
    }
}

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

/// Just for macro test
impl std::cmp::PartialEq<EncodedErr> for EncodedErr {
    fn eq(&self, other: &EncodedErr) -> bool {
        match self {
            Self::Rpc(e) => {
                if let Self::Rpc(o) = other {
                    return e == o;
                }
            }
            Self::Num(e) => {
                if let Self::Num(o) = other {
                    return e == o;
                }
            }
            Self::Static(s) => {
                if let Ok(o) = other.try_as_str() {
                    return *s == o;
                }
            }
            Self::Buf(s) => {
                if let Self::Buf(o) = other {
                    return s == o;
                } else if let Ok(o) = other.try_as_str() {
                    // other's type is not Buf
                    if let Ok(_s) = str::from_utf8(s) {
                        return _s == o;
                    }
                }
            }
        }
        false
    }
}

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

impl From<RpcIntErr> for EncodedErr {
    #[inline(always)]
    fn from(e: RpcIntErr) -> Self {
        Self::Rpc(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nix::errno::Errno;
    use std::str::FromStr;

    #[test]
    fn test_internal_error() {
        println!("{}", RpcIntErr::Internal);
        println!("{:?}", RpcIntErr::Internal);
        let s = RpcIntErr::Timeout.as_ref();
        println!("RpcIntErr::Timeout as {}", s);
        let e = RpcIntErr::from_str(s).expect("parse");
        assert_eq!(e, RpcIntErr::Timeout);
        assert!(RpcIntErr::from_str("timeoutss").is_err());
    }

    #[test]
    fn test_rpc_error_default() {
        let e = RpcError::<i32>::from(1i32);
        println!("err {:?} {}", e, e);

        let e = RpcError::<Errno>::from(Errno::EIO);
        println!("err {:?} {}", e, e);

        let e = RpcError::<String>::from("err_str");
        println!("err {:?} {}", e, e);

        let e = RpcError::<String>::from(RpcIntErr::IO);
        println!("err {:?} {}", e, e);

        let _e: Result<(), RpcIntErr> = Err(RpcIntErr::IO);

        // let e: Result<(), RpcError::<String>> = _e.into();
        // Not allow by rust, and orphan rule prevent we do
        // `impl<E: RpcErrCodec> From<Result<(), RpcIntErr>> for Result<(), RpcError<E>>`

        // it's ok to use map_err
        let e: Result<(), RpcError<String>> = _e.map_err(|e| e.into());
        println!("err {:?}", e);
    }

    //#[test]
    //fn test_rpc_error_string_enum() {
    //    not supported by default, should provide a derive
    //    #[derive(
    //        Debug, strum::Display, strum::EnumString, strum::AsRefStr, PartialEq, Clone, thiserror::Error,
    //    )]
    //    enum MyError {
    //        OhMyGod,
    //    }
    //    let e = RpcError::<MyError>::from(MyError::OhMyGod);
    //    println!("err {:?} {}", e, e);
    //}
}

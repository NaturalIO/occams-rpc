use crate::Codec;
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

impl<E: RpcErrCodec> fmt::Display for RpcError<E> {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::User(e) => RpcErrCodec::fmt(e, f),
            Self::Rpc(e) => fmt::Display::fmt(e, f),
        }
    }
}

impl<E: RpcErrCodec> fmt::Debug for RpcError<E> {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Display::fmt(self, f)
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

impl<E: RpcErrCodec + PartialEq> std::cmp::PartialEq<E> for RpcError<E> {
    #[inline]
    fn eq(&self, other: &E) -> bool {
        if let Self::User(r) = self {
            return r == other;
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

impl<E: RpcErrCodec> From<RpcIntErr> for RpcError<E> {
    #[inline]
    fn from(e: RpcIntErr) -> Self {
        Self::Rpc(e)
    }
}

/// Serialize and Deserialize trait for custom RpcError
///
/// There is only two forms for rpc transport layer, u32 and String, choose one of them.
///
/// Because Rust does not allow overlapping impl, we only imple RpcErrCodec trait by default for the following types:
/// - ()
/// - from i8 to u32
/// - String
/// - nix::errno::Errno
///
/// If you use other type as error, you can implement manually:
///
/// # Example
///
/// with serde_derive
/// ```rust
/// use serde_derive::{Serialize, Deserialize};
/// use occams_rpc_core::{error::{RpcErrCodec, RpcIntErr, EncodedErr}, Codec};
/// use strum::Display;
/// #[derive(Serialize, Deserialize, Debug)]
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
///
///     #[inline(always)]
///     fn decode<C: Codec>(codec: &C, buf: Result<u32, &[u8]>) -> Result<Self, ()> {
///         if let Err(b) = buf {
///             return codec.decode(b);
///         } else {
///             Err(())
///         }
///     }
///     #[inline(always)]
///     fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
///         std::fmt::Debug::fmt(self, f)
///     }
/// }
/// ```
pub trait RpcErrCodec: Send + Sized + 'static + Unpin {
    fn encode<C: Codec>(&self, codec: &C) -> EncodedErr;

    fn decode<C: Codec>(codec: &C, buf: Result<u32, &[u8]>) -> Result<Self, ()>;

    /// You can choose to use std::fmt::Debug or std::fmt::Display for the type.
    ///
    /// NOTE that this method exists because rust does not have Display for ().
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result;
}

macro_rules! impl_rpc_error_for_num {
    ($t: tt) => {
        impl RpcErrCodec for $t {
            #[inline(always)]
            fn encode<C: Codec>(&self, _codec: &C) -> EncodedErr {
                EncodedErr::Num(*self as u32)
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

            #[inline(always)]
            fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
                write!(f, "errno {}", self)
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
        EncodedErr::Num(*self as u32)
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

    #[inline(always)]
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl RpcErrCodec for () {
    #[inline(always)]
    fn encode<C: Codec>(&self, _codec: &C) -> EncodedErr {
        EncodedErr::Num(0u32)
    }

    #[inline(always)]
    fn decode<C: Codec>(_codec: &C, buf: Result<u32, &[u8]>) -> Result<Self, ()> {
        if let Ok(i) = buf {
            if i == 0 {
                return Ok(());
            }
        }
        Err(())
    }

    #[inline(always)]
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "err")
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

    #[inline(always)]
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self)
    }
}

/// "rpc_" prefix is reserved for internal error
///
/// NOTE Retriable error: RpcIntErr as u8 < RpcIntErr::Method
#[derive(
    strum::Display,
    strum::EnumString,
    strum::AsRefStr,
    PartialEq,
    PartialOrd,
    Clone,
    thiserror::Error,
)]
#[repr(u8)]
pub enum RpcIntErr {
    /// Ping or connect error
    #[strum(serialize = "rpc_unreachable")]
    Unreachable = 0,
    /// IO error
    #[strum(serialize = "rpc_io_err")]
    IO = 1,
    /// Task timeout
    #[strum(serialize = "rpc_timeout")]
    Timeout = 2,
    /// Method not found
    #[strum(serialize = "rpc_method_notfound")]
    Method = 3,
    /// service notfound
    #[strum(serialize = "rpc_service_notfound")]
    Service = 4,
    /// Encode Error
    #[strum(serialize = "rpc_encode")]
    Encode = 5,
    /// Decode Error
    #[strum(serialize = "rpc_decode")]
    Decode = 6,
    /// Internal error
    #[strum(serialize = "rpc_internal_err")]
    Internal = 7,
    /// invalid version number in rpc header
    #[strum(serialize = "rpc_invalid_ver")]
    Version = 8,
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
    Num(u32),
    /// only for server, the ClientTransport will not parse into static type
    Static(&'static str),
    /// The ClientTransport will fallback to `Vec<u8>` after try to parse  RpcIntErr and  num
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
        assert!(RpcIntErr::Timeout < RpcIntErr::Method);
        assert!(RpcIntErr::IO < RpcIntErr::Method);
        assert!(RpcIntErr::Unreachable < RpcIntErr::Method);
    }

    #[test]
    fn test_rpc_error_default() {
        let e = RpcError::<i32>::from(1i32);
        println!("err {:?} {}", e, e);

        let e = RpcError::<Errno>::from(Errno::EIO);
        println!("err {:?} {}", e, e);

        let e = RpcError::<String>::from("err_str");
        println!("err {:?} {}", e, e);
        let e2 = RpcError::<String>::from("err_str".to_string());
        assert_eq!(e, e2);

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

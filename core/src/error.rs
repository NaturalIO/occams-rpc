use std::convert::{TryFrom, TryInto};
use std::fmt;

pub const RPC_ERR_PREFIX: &'static str = "rpc_";

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

// Some method in stream requires returning &ServerErr
pub static RPC_ERR_ENCODE: ServerErr = ServerErr::Rpc(RpcIntErr::Encode);
pub static RPC_ERR_DECODE: ServerErr = ServerErr::Rpc(RpcIntErr::Decode);
// Reference from macros
pub static RPC_ERR_INTERNAL: ServerErr = ServerErr::Rpc(RpcIntErr::Internal);

/// A error type defined by client-side user logic
///
/// Due to possible decode
#[derive(thiserror::Error)]
pub enum RpcErrWrap<E: TryFrom<ServerErr>> {
    User(E),
    Rpc(&'static RpcIntErr),
}

impl<E: TryFrom<ServerErr>> std::cmp::PartialEq<RpcIntErr> for RpcErrWrap<E> {
    fn eq(&self, other: &RpcIntErr) -> bool {
        if let Self::Rpc(r) = self {
            if *r == other {
                return true;
            }
        }
        false
    }
}

/// A container for error message parse from / send into transport
#[derive(Debug, thiserror::Error)]
pub enum ServerErr {
    /// The ClientTransport should try the best to parse it from string with "rpc_" prefix
    Rpc(RpcIntErr),
    /// For nix errno and the like
    Num(i32),
    /// only for server, the ClientTransport will not parse into static type
    Str(&'static str),
    /// The ClientTransport would not try to parse it as utf8 string
    Text(String),
    /// The ClientTransport will fallback to Vec<u8> after try to parse  RpcIntErr and  num
    Buf(Vec<u8>),
}

impl ServerErr {
    #[inline]
    pub fn try_as_str<'a>(&'a self) -> Result<&'a str, ()> {
        match self {
            Self::Str(s) => return Ok(s),
            Self::Text(s) => return Ok(s.as_str()),
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

impl std::cmp::PartialEq<RpcIntErr> for ServerErr {
    fn eq(&self, other: &RpcIntErr) -> bool {
        if let Self::Rpc(r) = self {
            if r == other {
                return true;
            }
        }
        false
    }
}

impl std::cmp::PartialEq<ServerErr> for ServerErr {
    fn eq(&self, other: &ServerErr) -> bool {
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
            Self::Str(s) => {
                if let Ok(o) = other.try_as_str() {
                    return *s == o;
                }
            }
            Self::Text(s) => {
                if let Ok(o) = other.try_as_str() {
                    return s == o;
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

impl fmt::Display for ServerErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Rpc(e) => e.fmt(f),
            Self::Num(no) => write!(f, "errno {}", no),
            Self::Text(s) => write!(f, "{}", s),
            Self::Str(s) => write!(f, "{}", s),
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

/// For nix::errno
impl From<nix::errno::Errno> for ServerErr {
    #[inline(always)]
    fn from(e: nix::errno::Errno) -> Self {
        ServerErr::Num(e as i32)
    }
}

/// the same with errno
impl From<i32> for ServerErr {
    #[inline(always)]
    fn from(e: i32) -> Self {
        ServerErr::Num(e)
    }
}

impl From<&'static str> for ServerErr {
    #[inline(always)]
    fn from(s: &'static str) -> Self {
        ServerErr::Str(s)
    }
}

impl From<&String> for ServerErr {
    #[inline(always)]
    fn from(s: &String) -> Self {
        ServerErr::Text(s.to_string())
    }
}

impl From<String> for ServerErr {
    #[inline(always)]
    fn from(s: String) -> Self {
        ServerErr::Text(s)
    }
}

/// For nix::errno
impl Into<i32> for ServerErr {
    #[inline(always)]
    fn into(self) -> i32 {
        if let ServerErr::Num(i) = self {
            return i as i32;
        } else {
            unimplemented!();
        }
    }
}

impl From<RpcIntErr> for ServerErr {
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
    fn test_error_conversion() {
        println!("{}", RpcIntErr::Internal);
        println!("{:?}", RpcIntErr::Internal);
        let s = RpcIntErr::Timeout.as_ref();
        println!("RpcIntErr::Timeout as {}", s);
        let e = RpcIntErr::from_str(s).expect("parse");
        assert_eq!(e, RpcIntErr::Timeout);
        assert!(RpcIntErr::from_str("timeoutss").is_err());

        println!("test errno {:?}", Errno::EIO);
        let e: ServerErr = Errno::EIO.into();
        println!("{}", e);
        println!("{:?}", e);

        println!("test from str");
        let e = ServerErr::from("err_str");
        println!("{:?}", e);

        let e = ServerErr::from("err_str".to_string());
        println!("{:?}", e);
    }
}

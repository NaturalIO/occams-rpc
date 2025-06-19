use nix::errno::Errno;
use std::fmt;

// All "rpc_" prefix error is internal error of RpcError::Rpc()
pub const ERR_CLOSED: &'static str = "rpc_close";
pub const ERR_NOT_SUPPORTED: &'static str = "rpc_format_not_supported";
pub const ERR_INVALID_BODY: &'static str = "rpc_invalid_body";
pub const ERR_INVALID_METHOD: &'static str = "rpc_invalid_method";
pub const ERR_CONNECT: &'static str = "rpc_connect_error";
pub const ERR_TIMEOUT: &'static str = "rpc_timeout";
pub const ERR_COMM: &'static str = "rpc_error";
pub const ERR_FAILOVER: &'static str = "rpc_failover";
pub const ERR_ENCODE: &'static str = "rpc_encode_error";
pub const ERR_DECODE: &'static str = "rpc_decode_error";

pub const RPC_ERR_TIMEOUT: RpcError = RpcError::Rpc(ERR_TIMEOUT);
pub const RPC_ERR_CONNECT: RpcError = RpcError::Rpc(ERR_CONNECT);
pub const RPC_ERR_COMM: RpcError = RpcError::Rpc(ERR_COMM);
pub const RPC_ERR_CLOSED: RpcError = RpcError::Rpc(ERR_CLOSED);
pub const RPC_ERR_ENCODE: RpcError = RpcError::Rpc(ERR_ENCODE);
pub const RPC_ERR_DECODE: RpcError = RpcError::Rpc(ERR_DECODE);

#[derive(Clone, Debug, PartialEq)]
pub enum RpcError {
    Posix(Errno),
    Rpc(&'static str),
    Remote(String),
}

impl fmt::Display for RpcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Posix(e) => write!(f, "{}", e.desc()),
            Self::Rpc(s) => write!(f, "error: {}", s),
            Self::Remote(s) => write!(f, "Remote error: {}", s),
        }
    }
}

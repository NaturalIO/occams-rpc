use super::service::*;
use nix::errno::Errno;
use occams_rpc::server::service;
use occams_rpc_core::error::RpcError;

#[derive(Clone)]
pub struct CalServer();

#[service]
#[async_trait::async_trait]
impl CalService for CalServer {
    async fn inc(&self, y: isize) -> Result<isize, RpcError<()>> {
        Ok(y + 1)
    }

    async fn add(&self, args: (isize, isize)) -> Result<isize, RpcError<()>> {
        let (a, b) = args;
        Ok(a + b)
    }

    async fn div(&self, args: (isize, isize)) -> Result<isize, RpcError<String>> {
        let (a, b) = args;
        if b == 0 {
            return Err(RpcError::User("divide by zero".to_string()));
        }
        return Ok(a / b);
    }
}

#[derive(Clone)]
pub struct EchoServer();

#[service]
impl EchoService for EchoServer {
    async fn repeat(&self, msg: String) -> Result<String, RpcError<()>> {
        return Ok(msg);
    }

    async fn io_error(&self, _msg: String) -> Result<(), RpcError<Errno>> {
        return Err(RpcError::User(Errno::EIO));
    }
}

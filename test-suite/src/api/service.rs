use nix::errno::Errno;
use occams_rpc::client::endpoint_async;
use occams_rpc_core::error::RpcError;

#[endpoint_async(CalClient)]
#[async_trait::async_trait]
pub trait CalService {
    async fn inc(&self, y: isize) -> Result<isize, RpcError<()>>;

    async fn add(&self, args: (isize, isize)) -> Result<isize, RpcError<()>>;

    async fn div(&self, args: (isize, isize)) -> Result<isize, RpcError<String>>;
}

#[endpoint_async(EchoClient)]
pub trait EchoService {
    fn repeat(&self, msg: String) -> impl Future<Output = Result<String, RpcError<()>>> + Send;

    fn io_error(&self, _msg: String) -> impl Future<Output = Result<(), RpcError<Errno>>> + Send;
}

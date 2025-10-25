pub use super::service::{CalClient, EchoClient};
use occams_rpc::client::*;
use occams_rpc_core::{ClientConfig, Codec};
use occams_rpc_tcp::TcpClient;

#[cfg(not(feature = "tokio"))]
use occams_rpc_smol::{ClientDefault, SmolRT};
#[cfg(feature = "tokio")]
use occams_rpc_tokio::{ClientDefault, TokioRT};

#[cfg(feature = "tokio")]
pub type APIClientDefault<C> = occams_rpc_tokio::ClientDefault<APIClientReq, C>;
#[cfg(all(not(feature = "tokio"), feature = "smol"))]
pub type APIClientDefault<C> = occams_rpc_smol::ClientDefault<APIClientReq, C>;

pub type PoolCaller<C> = ClientPool<APIClientDefault<C>, TcpClient<crate::RT>>;

pub struct MyClient<C: Codec> {
    pub cal: CalClient<PoolCaller<C>>,
    pub echo: EchoClient<PoolCaller<C>>,
}

impl<C: Codec> MyClient<C> {
    pub fn new(config: ClientConfig, addr: &str) -> Self {
        #[cfg(feature = "tokio")]
        let rt = TokioRT::new(tokio::runtime::Handle::current());
        #[cfg(not(feature = "tokio"))]
        let rt = SmolRT::new_global();
        let facts = ClientDefault::<APIClientReq, C>::new(config, rt);
        let pool = facts.clone().create_pool_async::<TcpClient<crate::RT>>(addr);
        let cal = CalClient::new(pool.clone());
        let echo = EchoClient::new(pool.clone());
        MyClient { cal, echo }
    }
}

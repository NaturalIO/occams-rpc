pub use super::service::{CalClient, EchoClient};
use razor_rpc::client::*;
use razor_rpc_core::{ClientConfig, Codec};
use razor_rpc_tcp::TcpClient;

use crate::ClientRT;
#[cfg(not(feature = "tokio"))]
use orb_smol::SmolRT;
#[cfg(feature = "tokio")]
use orb_tokio::TokioRT;
#[cfg(feature = "tokio")]
use razor_rpc_stream::client::ClientDefault;
#[cfg(not(feature = "tokio"))]
use razor_stream::client::ClientDefault;

#[cfg(feature = "tokio")]
pub type APIClientDefault<C> =
    razor_stream::client::ClientDefault<APIClientReq, crate::ClientRT<orb_tokio::TokioRT>, C>;
#[cfg(all(not(feature = "tokio"), feature = "smol"))]
pub type APIClientDefault<C> =
    razor_stream::client::ClientDefault<APIClientReq, crate::ClientRT<orb_smol::SmolRT>, C>;

pub type PoolCaller<C> = ClientPool<APIClientDefault<C>, TcpClient<crate::ClientRT<crate::RT>>>;

pub struct MyClient<C: Codec> {
    pub cal: CalClient<PoolCaller<C>>,
    pub echo: EchoClient<PoolCaller<C>>,
}

impl<C: Codec> MyClient<C> {
    pub fn new(config: ClientConfig, addr: &str) -> Self {
        #[cfg(feature = "tokio")]
        let rt = ClientRT(TokioRT::new(tokio::runtime::Handle::current()));
        #[cfg(not(feature = "tokio"))]
        let rt = ClientRT(SmolRT::new_global());
        let facts = ClientDefault::<APIClientReq, crate::ClientRT<crate::RT>, C>::new(config, rt);
        let pool = facts.clone().create_pool_async::<TcpClient<crate::RT>>(addr);
        let cal = CalClient::new(pool.clone());
        let echo = EchoClient::new(pool.clone());
        MyClient { cal, echo }
    }
}

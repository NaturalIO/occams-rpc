pub use super::service::{CalClient, EchoClient};
use razor_rpc::client::*;
use razor_rpc_core::{ClientConfig, Codec};
use razor_rpc_tcp::TcpClient;

use razor_stream::client::ClientDefault;

pub type APIClient<C> = razor_stream::client::ClientDefault<APIClientReq, crate::RT, C>;

pub type PoolCaller<C> = ClientPool<APIClient<C>, TcpClient<crate::RT>>;

pub struct MyClient<C: Codec> {
    pub cal: CalClient<PoolCaller<C>>,
    pub echo: EchoClient<PoolCaller<C>>,
}

impl<C: Codec> MyClient<C> {
    pub fn new(config: ClientConfig, addr: &str) -> Self {
        let facts = APIClient::<C>::new(config, crate::new_rt());
        let pool = facts.clone().create_pool_async::<TcpClient<crate::RT>>(addr);
        let cal = CalClient::new(pool.clone());
        let echo = EchoClient::new(pool.clone());
        MyClient { cal, echo }
    }
}

use super::service::*;
use nix::errno::Errno;
use occams_rpc::server::service;
use occams_rpc_codec::MsgpCodec;
use occams_rpc_core::error::RpcError;
use occams_rpc_stream::server::{RpcServer, ServerConfig};
use occams_rpc_tcp::TcpServer;
use rstest::*;
use std::sync::Arc;

#[derive(Clone, Debug)]
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

#[derive(Clone, Debug)]
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

// Create an API server with the given services
pub fn create_api_server(
    config: ServerConfig,
) -> RpcServer<occams_rpc::server::ServerDefault<crate::RT>> {
    use occams_rpc::server::ServerDefault;

    #[cfg(feature = "tokio")]
    let rt = crate::RT::new(tokio::runtime::Handle::current());
    #[cfg(not(feature = "tokio"))]
    let rt = crate::RT::new_global();

    let facts = ServerDefault::new(config, rt);
    let server = RpcServer::new(facts);

    server
}

// Add services to the server and start listening
pub fn listen_with_services(
    mut server: RpcServer<occams_rpc::server::ServerDefault<crate::RT>>, bind_addr: &str,
    cal_server: CalServer, echo_server: EchoServer,
) -> Result<
    (RpcServer<occams_rpc::server::ServerDefault<crate::RT>>, String),
    Box<dyn std::error::Error>,
> {
    use occams_rpc::server::ServiceMuxDyn;
    use occams_rpc::server::dispatch::Inline;

    // Create service mux and add services
    let mut service_mux = ServiceMuxDyn::<MsgpCodec>::new();
    service_mux.add(Arc::new(cal_server));
    service_mux.add(Arc::new(echo_server));

    // Create dispatcher
    let dispatch = Inline::new(Arc::new(service_mux));

    // Listen on the address
    let actual_addr = server.listen::<TcpServer<crate::RT>, _>(bind_addr, dispatch)?;

    Ok((server, actual_addr))
}

// Fixture for service_mux_struct testing
#[fixture]
pub fn cal_server() -> CalServer {
    CalServer {}
}

#[fixture]
pub fn echo_server() -> EchoServer {
    EchoServer {}
}

// Create service mux dynamic dispatch
pub fn create_service_mux_dispatch(
    cal_server: CalServer, echo_server: EchoServer,
) -> impl occams_rpc_stream::server::dispatch::Dispatch {
    use occams_rpc::server::{ServiceMuxDyn, dispatch::Inline};

    let mut service_mux = ServiceMuxDyn::<MsgpCodec>::new();
    service_mux.add(Arc::new(cal_server));
    service_mux.add(Arc::new(echo_server));

    Inline::new(Arc::new(service_mux))
}

// Create service mux struct dispatch
pub fn create_service_mux_struct_dispatch(
    cal_server: CalServer, echo_server: EchoServer,
) -> impl occams_rpc_stream::server::dispatch::Dispatch {
    use occams_rpc::server::dispatch::Inline;
    use occams_rpc::server::service_mux_struct;
    use std::sync::Arc;

    #[service_mux_struct]
    #[derive(Clone)]
    struct TestServiceMux {
        cal: Arc<CalServer>,
        echo: Arc<EchoServer>,
    }

    let service_mux = TestServiceMux { cal: Arc::new(cal_server), echo: Arc::new(echo_server) };

    Inline::<MsgpCodec, TestServiceMux>::new(service_mux)
}

// Fixture that returns a service mux dispatch
#[fixture]
pub fn service_mux_dispatch() -> occams_rpc::server::ServiceMuxDyn<MsgpCodec> {
    use occams_rpc::server::ServiceMuxDyn;

    // Create service mux
    ServiceMuxDyn::<MsgpCodec>::new()
}

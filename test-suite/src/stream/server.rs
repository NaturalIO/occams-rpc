use super::client::{FileAction, FileIOReq, FileIOResp, FileOpenReq};
use nix::errno::Errno;
use occams_rpc_codec::MsgpCodec;
#[cfg(not(feature = "tokio"))]
use occams_rpc_smol::{ServerDefault, SmolRT};
use occams_rpc_stream::server::{dispatch::*, task::*, *};
use occams_rpc_tcp::TcpServer;
#[cfg(feature = "tokio")]
use occams_rpc_tokio::{ServerDefault, TokioRT};

pub fn init_server(config: ServerConfig) -> RpcServer<ServerDefault> {
    #[cfg(feature = "tokio")]
    let rt = TokioRT::new(tokio::runtime::Handle::current());
    #[cfg(not(feature = "tokio"))]
    let rt = SmolRT::new_global();
    let facts = ServerDefault::new(config, rt);
    RpcServer::new(facts)
}

pub fn init_server_closure<H, FH>(
    server_handle: H, config: ServerConfig, addr: &str,
) -> Result<(RpcServer<ServerDefault>, String), std::io::Error>
where
    H: FnOnce(FileServerTask) -> FH + Send + Sync + 'static + Clone,
    FH: Future<Output = Result<(), ()>> + Send + 'static,
{
    let mut server = init_server(config);
    let dispatch = new_closure_dispatcher(server_handle);
    let local_addr = server.listen::<TcpServer<crate::RT>, _>(addr, dispatch)?;
    Ok((server, local_addr))
}

pub fn new_closure_dispatcher<H, FH>(handle: H) -> impl Dispatch
where
    H: FnOnce(FileServerTask) -> FH + Send + Sync + 'static + Clone,
    FH: Future<Output = Result<(), ()>> + Send + 'static,
{
    DispatchClosure::<MsgpCodec, FileServerTask, FileServerTask, _, _>::new(handle.clone())
}

#[server_task_enum(req, resp, error = Errno)]
#[derive(Debug)]
pub enum FileServerTask {
    #[action(FileAction::Open)]
    Open(ServerTaskOpen),
    #[action(FileAction::Read, FileAction::Write)]
    IO(ServerTaskIO),
}

pub type ServerTaskOpen = ServerTaskVariantFull<FileServerTask, FileOpenReq, (), Errno>;
pub type ServerTaskIO = ServerTaskVariantFull<FileServerTask, FileIOReq, FileIOResp, Errno>;

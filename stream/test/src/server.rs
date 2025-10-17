use super::client::{FileAction, FileIOReq, FileIOResp, FileOpenReq};
use occams_rpc_codec::MsgpCodec;
use occams_rpc_stream::server::*;
use occams_rpc_stream::server_impl::*;
use std::sync::Arc;

use occams_rpc_stream::macros::*;

use captains_log::filter::LogFilter;
use nix::errno::Errno;

pub fn init_server<H, FH>(
    server_handle: H, config: ServerConfig, addr: &str,
) -> Result<(RpcServer<FileServer<H, FH>>, String), std::io::Error>
where
    H: FnOnce(FileServerTask) -> FH + Send + Sync + 'static + Clone,
    FH: Future<Output = Result<(), ()>> + Send + 'static,
{
    let factory = Arc::new(FileServer::new(server_handle, config));
    let mut server = RpcServer::new(factory);
    let local_addr = server.listen(addr)?;
    Ok((server, local_addr))
}

pub struct FileServer<H, FH>
where
    H: FnOnce(FileServerTask) -> FH + Send + Sync + 'static + Clone,
    FH: Future<Output = Result<(), ()>> + Send + 'static,
{
    config: ServerConfig,
    server_handle: H,
    logger: Arc<LogFilter>,
}

impl<H, FH> FileServer<H, FH>
where
    H: FnOnce(FileServerTask) -> FH + Send + Sync + 'static + Clone,
    FH: Future<Output = Result<(), ()>> + Send + 'static,
{
    pub fn new(server_handle: H, config: ServerConfig) -> Self {
        Self { config, server_handle, logger: Arc::new(LogFilter::new()) }
    }
}

impl<H, FH> ServerFactory for FileServer<H, FH>
where
    H: FnOnce(FileServerTask) -> FH + Send + Sync + 'static + Clone,
    FH: Future<Output = Result<(), ()>> + Send + 'static,
{
    type Logger = Arc<LogFilter>;

    type Transport = occams_rpc_tcp::TcpServer<Self>;

    type IO = crate::RT;

    type RespReceiver = RespReceiverTask<FileServerTask>;

    #[inline]
    fn new_dispatcher(&self) -> impl ReqDispatch<Self::RespReceiver> {
        return ReqDispatchClosure::<MsgpCodec, FileServerTask, Self::RespReceiver, _, _>::new(
            self.server_handle.clone(),
        );
    }

    #[inline]
    fn spawn_detach<F, R>(&self, f: F)
    where
        F: Future<Output = R> + Send + 'static,
        R: Send + 'static,
    {
        crate::async_spawn!(f);
    }

    #[inline]
    fn new_logger(&self) -> Self::Logger {
        self.logger.clone()
    }

    #[inline]
    fn get_config(&self) -> &ServerConfig {
        &self.config
    }
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

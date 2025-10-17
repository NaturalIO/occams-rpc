/// This test is to be sure that macro use full path calls without any trait imported
#[test]
#[allow(dead_code)]
fn test_client_task_define() {
    use crossfire::MTx;
    use nix::errno::Errno;
    use occams_rpc_core::error::RpcError;
    use occams_rpc_stream::client::ClientTaskCommon;
    use occams_rpc_stream_macros::client_task;
    use serde_derive::{Deserialize, Serialize};

    #[derive(Default, Deserialize, Serialize, Debug, PartialEq, Clone)]
    pub struct FileIOReq {
        pub path: String,
        pub offset: u64,
    }

    #[derive(Default, Deserialize, Serialize, Debug, PartialEq)]
    pub struct FileIOResp {
        pub bytes_read: u64,
    }

    #[derive(PartialEq)]
    #[repr(u8)]
    enum FileAction {
        Read = 1,
        Write = 2,
    }

    #[client_task(FileAction::Write, debug)]
    pub struct FileWriteTask {
        #[field(common)]
        common: ClientTaskCommon,
        #[field(req)]
        req: FileIOReq,
        #[field(req_blob)]
        req_blob: Vec<u8>,
        #[field(resp)]
        resp: Option<FileIOResp>,
        #[field(res)]
        res: Option<Result<(), RpcError<Errno>>>,
        #[field(noti)]
        noti: Option<MTx<Self>>,
    }

    #[client_task(FileAction::Read, debug)]
    pub struct FileReadTask {
        #[field(common)]
        common: ClientTaskCommon,
        #[field(req)]
        req: FileIOReq,
        #[field(resp)]
        resp: Option<FileIOResp>,
        #[field(resp_blob)]
        resp_blob: Option<Vec<u8>>,
        #[field(res)]
        res: Option<Result<(), RpcError<Errno>>>,
        #[field(noti)]
        noti: Option<MTx<Self>>,
    }
}

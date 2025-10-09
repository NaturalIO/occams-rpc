/// This test is to be sure that macro use full path calls without any trait imported
#[test]
#[allow(dead_code)]
fn test_client_task_enum_define() {
    use crossfire::MTx;
    use occams_rpc::error::RpcError;
    use occams_rpc::stream::client::ClientTaskCommon;
    use occams_rpc_macros::{client_task, client_task_enum};

    #[derive(PartialEq)]
    #[repr(u8)]
    enum FileAction {
        Open = 1,
        Close = 2,
    }

    #[client_task(debug)]
    pub struct FileOpenTask {
        #[field(common)]
        common: ClientTaskCommon,
        #[field(req)]
        req: String,
        #[field(resp)]
        resp: Option<()>,
        #[field(res)]
        res: Option<Result<(), RpcError>>,
        #[field(noti)]
        noti: Option<MTx<FileTask>>,
    }

    #[client_task(2, debug)] // This action will be used as the variant doesn't specify one
    pub struct FileCloseTask {
        #[field(common)]
        common: ClientTaskCommon,
        #[field(req)]
        req: (),
        #[field(resp)]
        resp: Option<()>,
        #[field(res)]
        res: Option<Result<(), RpcError>>,
        #[field(noti)]
        noti: Option<MTx<FileTask>>,
    }

    #[client_task_enum]
    #[derive(Debug)]
    pub enum FileTask {
        #[action(FileAction::Open)]
        Open(FileOpenTask),
        // This variant delegates action to the inner type
        Close(FileCloseTask),
    }
}

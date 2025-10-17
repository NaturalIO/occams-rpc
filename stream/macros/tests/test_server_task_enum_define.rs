/// This test is to be sure that macro use full path calls without any trait imported
#[test]
#[allow(dead_code)]
fn test_server_task_enum_define() {
    use nix::errno::Errno;
    use occams_rpc_stream::server_impl::ServerTaskVariantFull;
    use occams_rpc_stream_macros::server_task_enum;
    use serde_derive::{Deserialize, Serialize};

    #[derive(Serialize, Deserialize, Debug, Default, PartialEq)]
    struct FileOpenReq {
        pub path: String,
    }

    #[derive(Serialize, Deserialize, Debug, Default, PartialEq)]
    struct FileReadReq {
        pub path: String,
        pub offset: u64,
        pub len: u64,
    }

    #[derive(PartialEq)]
    #[repr(u8)]
    enum FileAction {
        Open = 1,
        Read = 2,
    }

    #[server_task_enum(req, resp, error=Errno)]
    #[derive(Debug)]
    pub enum FileServerTask {
        #[action(1)]
        Open(ServerTaskVariantFull<FileServerTask, FileOpenReq, (), Errno>),
        #[action(2)]
        Read(ServerTaskVariantFull<FileServerTask, FileReadReq, (), Errno>),
    }
}

use captains_log::recipe;
use crossfire::mpsc::*;
use display_attr::*;
use io_engine::buffer::Buffer;
use log::*;
use nix::errno::Errno;
use occams_rpc::stream::client::*;
use occams_rpc::stream::client_task::*;
use occams_rpc::stream::*;
use occams_rpc::*;
use serde_derive::{Deserialize, Serialize};
use std::fmt;
use tokio::runtime::{Builder, Runtime};

pub fn setup() -> Runtime {
    recipe::raw_file_logger("/tmp", "rpc_test", Level::Trace).test().build();
    let rt = Builder::new_multi_thread().worker_threads(8).enable_all().build().unwrap();
    rt
}

#[derive(Default, Deserialize, Serialize, DisplayAttr)]
#[display(fmt = "inode={} offset={}", inode, offset)]
pub struct FileIOReq {
    #[serde(rename = "i")]
    pub inode: u64,
    #[serde(rename = "o")]
    pub offset: i64,
}

#[derive(Default, Deserialize, Serialize, DisplayAttr)]
#[display(fmt = "ret={}", ret)]
pub struct FileIOResp {
    #[serde(rename = "r")]
    pub ret: i64, // read or write ret
}

macro_rules! unwrap_msg {
    ($seq: expr, $action: expr, $msg: expr, $req_cls: tt) => {
        if let Some(_msg) = $msg {
            match rmp_serde::decode::from_slice::<$req_cls>(_msg) {
                Err(e) => {
                    println!("seq={} action={} msg decode error: {:#?}", $seq, $action, e);
                    Err(())
                }
                Ok(_msg) => {
                    let task = TestServerTask { seq: $seq, action: $action };
                    Ok(task)
                }
            }
        } else {
            println!("seq={} action={} missing msg field", $seq, $action);
            Err(())
        }
    };
}

pub struct TestServerTask {
    seq: u64,
    action: i32,
}

pub fn test_task_decoder(
    seq: u64, action: i32, msg: Option<&[u8]>, _ext: Option<Buffer>,
) -> Result<TestServerTask, ()> {
    unwrap_msg!(seq, action, msg, FileIOReq)
}

pub struct FileTask {
    common: ClientTaskCommon,
    sender: TxUnbounded<Self>,
    req: FileIOReq,
    action: i32,
    res: Option<Result<(), RpcError>>,
}

impl fmt::Display for FileTask {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "FileTask seq={} action={}", self.common.seq, self.action)
    }
}

macro_rules! encode_task_msgp {
    ($req: expr) => {{
        match rmp_serde::encode::to_vec_named(&$req) {
            Ok(msg) => {
                return Some(msg);
            }
            Err(e) => {
                println!("{} encode error: {}", $req, e);
                return None;
            }
        }
    }};
}

impl FileTask {
    pub fn new(action: i32, sender: TxUnbounded<Self>) -> Self {
        Self {
            common: ClientTaskCommon::default(),
            sender,
            action,
            res: None,
            req: FileIOReq { inode: 1, offset: 0 },
        }
    }
}

impl_client_task_common!(FileTask, common);
impl_client_task_encode!(FileTask, req);

impl ClientTaskDecode for FileTask {
    fn set_result(mut self, res: Result<&[u8], RpcError>) {
        debug!("client recv result {} {:?}", self, res);
        match res {
            Ok(_) => {
                self.res = Some(Ok(()));
            }
            Err(e) => {
                self.res = Some(Err(e));
            }
        }
        let sender = self.sender.clone();
        sender.send(self);
    }
}

impl RpcClientTask for FileTask {
    fn action(&self) -> RpcAction {
        return RpcAction::Num(self.action);
    }
}

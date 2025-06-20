use super::RpcAction;
use crate::error::*;
use std::fmt::Display;
use std::ops::{Deref, DerefMut};

#[derive(Debug, Default)]
pub struct ClientTaskCommon {
    pub seq: u64,
}

impl ClientTaskCommon {
    pub fn seq(&self) -> u64 {
        self.seq
    }
    pub fn set_seq(&mut self, seq: u64) {
        self.seq = seq;
    }
}

pub trait RpcClientTask:
    ClientTaskEncode
    + ClientTaskDecode
    + Deref<Target = ClientTaskCommon>
    + DerefMut<Target = ClientTaskCommon>
    + Send
    + Sync
    + Sized
    + 'static
    + Display
    + Unpin
{
    fn action<'a>(&'a self) -> RpcAction<'a>;

    fn set_result(self, res: Result<&[u8], RpcError>);
}

pub struct RetryTaskInfo<T: RpcClientTask + Send + Unpin + 'static> {
    pub task: T,
    pub task_err: RpcError,
}

pub trait ClientTaskEncode {
    /// Return a sererialized msg of the request.
    fn get_req_msg_buf(&self) -> Option<Vec<u8>>;

    #[inline(always)]
    /// Contain optional extra data to send to server side.
    fn get_req_ext_buf(&self) -> Option<&[u8]> {
        None
    }
}

/// Macro to impl Deref<Target=ClientTaskCommon> and DerefMut for a struct
///
/// # example:
///
/// ``` rust
/// use occams_rpc::stream::client_task::*;
/// pub struct FileTask {
///    common: ClientTaskCommon,
/// }
/// impl_client_task_common!(FileTask, common);
/// ```
#[macro_export]
macro_rules! impl_client_task_common {
    ($cls: tt, $common_field: tt) => {
        impl std::ops::Deref for $cls {
            type Target = ClientTaskCommon;
            fn deref(&self) -> &Self::Target {
                &self.$common_field
            }
        }
        impl std::ops::DerefMut for $cls {
            fn deref_mut(&mut self) -> &mut Self::Target {
                &mut self.$common_field
            }
        }
    };
}
// export macro current level
//#[allow(unused_imports)]
pub use impl_client_task_common;

#[macro_export]
macro_rules! impl_client_task_encode {
    ($cls: tt, $req_field: tt) => {
        impl ClientTaskEncode for $cls {
            fn get_req_msg_buf(&self) -> Option<Vec<u8>> {
                todo!();
            }
        }
    };
    ($cls: tt, $req_field: tt, $blob_field: tt) => {
        impl ClientTaskEncode for $cls {
            fn get_req_msg_buf(&self) -> Option<Vec<u8>> {
                todo!();
            }

            fn get_req_ext_buf(&self) -> Option<&u8> {
                Some(self.$blob_field.as_ref())
            }
        }
    };
}
// export macro current level
#[allow(unused_imports)]
pub use impl_client_task_encode;

pub trait ClientTaskDecode {
    /// Get a mut buffer ref for large blob to hold optional extra data response from server
    ///
    /// `blob_len` is from RespHead, this interface should make sure that buffer of such size allocated.
    #[inline(always)]
    fn get_resp_ext_buf_mut(&mut self, _blob_len: i32) -> Option<&mut [u8]> {
        None
    }

    fn decode(&mut self, res: Result<&[u8], RpcError>) -> Result<(), RpcError>;
}

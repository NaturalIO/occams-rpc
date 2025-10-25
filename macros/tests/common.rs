use occams_rpc::server::task::{APIServerReq, APIServerResp};
use occams_rpc_api_macros::{method, service, service_mux_struct};
use occams_rpc_codec::MsgpCodec;
use occams_rpc_core::{error::RpcError, Codec};
use occams_rpc_stream::server::task::RespNoti;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub fn create_mock_request<T: Serialize>(
    seq: u64, service: String, method: String, req: &T, noti: RespNoti<APIServerResp>,
) -> APIServerReq<MsgpCodec> {
    let codec = Arc::new(MsgpCodec::default());
    let req_data = codec.encode(req).expect("encode");
    return APIServerReq { seq, service, method, req: Some(req_data), codec, noti };
}

#[derive(Debug, Deserialize, Serialize, PartialEq)]
pub struct MyArg {
    pub value: u32,
}

#[derive(Debug, Deserialize, Serialize, PartialEq)]
pub struct MyResp {
    pub result: u32,
}

// Service with multiple error types
pub struct MultiErrorServiceImpl;

#[service]
impl MultiErrorServiceImpl {
    #[method]
    async fn success_method(&self, arg: MyArg) -> Result<MyResp, RpcError<String>> {
        Ok(MyResp { result: arg.value + 1 })
    }

    #[method]
    async fn string_error(&self, _arg: MyArg) -> Result<MyResp, RpcError<String>> {
        Err("string error".to_string().into())
    }

    #[method]
    async fn i32_error(&self, _arg: MyArg) -> Result<MyResp, RpcError<i32>> {
        Err(42.into())
    }

    #[method]
    async fn errno_error(&self, _arg: MyArg) -> Result<MyResp, RpcError<nix::errno::Errno>> {
        Err(nix::errno::Errno::EPERM.into())
    }
}

// Service with `impl Future` return type (non-async fn)
pub struct ImplFutureService;
#[service]
impl ImplFutureService {
    #[method]
    pub fn add(
        &self, arg: MyArg,
    ) -> impl std::future::Future<Output = Result<MyResp, RpcError<String>>> + Send {
        async move { Ok(MyResp { result: arg.value + 1 }) }
    }
}

// Service using async_trait
#[async_trait::async_trait]
pub trait MyAsyncTraitService {
    async fn mul(&self, arg: MyArg) -> Result<MyResp, RpcError<String>>;
}
pub struct MyAsyncTraitServiceImpl;
#[async_trait::async_trait]
#[service]
impl MyAsyncTraitService for MyAsyncTraitServiceImpl {
    async fn mul(&self, arg: MyArg) -> Result<MyResp, RpcError<String>> {
        Ok(MyResp { result: arg.value * 2 })
    }
}

// Service Dispatcher Struct
#[service_mux_struct]
pub struct MyServices {
    pub multi: Arc<MultiErrorServiceImpl>,
    pub impl_future: Arc<ImplFutureService>,
}

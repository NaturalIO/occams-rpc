use occams_rpc::server::{Request, Response, ServiceTrait};
use occams_rpc_api_macros::{method, service, service_mux_struct};
use occams_rpc_codec::MsgpCodec;
use occams_rpc_core::Codec;
use occams_rpc_stream::server::RespNoti;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub fn create_mock_request<T: Serialize>(
    seq: u64, service: String, method: String, req: &T, noti: RespNoti<Response>,
) -> Request<MsgpCodec> {
    let codec = Arc::new(MsgpCodec::default());
    let req_data = codec.encode(req).expect("encode");
    return Request { seq, service, method, req: Some(req_data), codec, noti };
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
    async fn success_method(&self, arg: MyArg) -> Result<MyResp, String> {
        Ok(MyResp { result: arg.value + 1 })
    }

    #[method]
    async fn string_error(&self, _arg: MyArg) -> Result<MyResp, String> {
        Err("string error".to_string())
    }

    #[method]
    async fn i32_error(&self, _arg: MyArg) -> Result<MyResp, i32> {
        Err(42)
    }

    #[method]
    async fn errno_error(&self, _arg: MyArg) -> Result<MyResp, nix::errno::Errno> {
        Err(nix::errno::Errno::EPERM)
    }
}

// Service with `impl Future` return type (non-async fn)
pub struct ImplFutureServiceImpl;
#[service]
impl ImplFutureServiceImpl {
    #[method]
    pub fn add(
        &self, arg: MyArg,
    ) -> impl std::future::Future<Output = Result<MyResp, String>> + Send {
        async move { Ok(MyResp { result: arg.value + 1 }) }
    }
}

// Service using async_trait
#[async_trait::async_trait]
pub trait MyAsyncTraitService {
    async fn mul(&self, arg: MyArg) -> Result<MyResp, String>;
}
pub struct MyAsyncTraitServiceImpl;
#[async_trait::async_trait]
#[service]
impl MyAsyncTraitService for MyAsyncTraitServiceImpl {
    async fn mul(&self, arg: MyArg) -> Result<MyResp, String> {
        Ok(MyResp { result: arg.value * 2 })
    }
}

// Service Dispatcher Struct
#[service_mux_struct]
pub struct MyServices {
    pub multi: Arc<MultiErrorServiceImpl>,
    pub impl_future: Arc<ImplFutureServiceImpl>,
}

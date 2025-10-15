use occams_rpc::server::{Request, Response, ServiceTrait};
use occams_rpc_api_macros::{method, service, service_enum};
use occams_rpc_codec::MsgpCodec;
use occams_rpc_core::{error::RpcError, Codec};
use occams_rpc_stream::server::RespNoti;
use serde::{Deserialize, Serialize};
use std::sync::Arc; // Add this import

pub fn create_mock_request<T: Serialize>(
    seq: u64, service: String, method: String, req: &T, noti: RespNoti<Response>,
) -> Request<MsgpCodec> {
    let codec = Arc::new(MsgpCodec::default());
    let req_data = codec.encode(req).expect("encode");
    return Request { seq, service, method, req: Some(req_data), codec, noti };
}

pub struct MyServiceImpl;

#[derive(Debug, Deserialize, Serialize)]
pub struct MyArg {
    pub value: u32,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct MyResp {
    pub result: u32,
}

pub trait MyService {
    fn add(&self, arg: MyArg)
        -> impl std::future::Future<Output = Result<MyResp, RpcError>> + Send;
    fn sub(&self, arg: MyArg)
        -> impl std::future::Future<Output = Result<MyResp, RpcError>> + Send;
    fn always_error(
        &self, arg: MyArg,
    ) -> impl std::future::Future<Output = Result<MyResp, RpcError>> + Send;
}

#[service]
impl MyService for MyServiceImpl {
    async fn add(&self, arg: MyArg) -> Result<MyResp, RpcError> {
        Ok(MyResp { result: arg.value + 1 })
    }

    async fn sub(&self, arg: MyArg) -> Result<MyResp, RpcError> {
        Ok(MyResp { result: arg.value - 1 })
    }

    async fn always_error(&self, _arg: MyArg) -> Result<MyResp, RpcError> {
        Err(occams_rpc_core::error::RpcError::Text(
            "This method always returns an error".to_string(),
        ))
    }
}

#[service_enum]
pub enum MyServices {
    AddService(Arc<MyServiceImpl>),
    SubService(Arc<MyServiceImpl>),
}

#[async_trait::async_trait]
pub trait MyAsyncTraitService {
    async fn mul(&self, arg: MyArg) -> Result<MyResp, RpcError>;
    fn div(&self, arg: MyArg) -> Result<MyResp, RpcError>; // Non-async method
}

pub struct MyAsyncTraitServiceImpl;

#[async_trait::async_trait]
#[service]
impl MyAsyncTraitService for MyAsyncTraitServiceImpl {
    async fn mul(&self, arg: MyArg) -> Result<MyResp, RpcError> {
        Ok(MyResp { result: arg.value * 2 })
    }

    fn div(&self, arg: MyArg) -> Result<MyResp, RpcError> {
        Ok(MyResp { result: arg.value / 2 })
    }
}

pub struct MyServiceInherentImpl;

#[service]
impl MyServiceInherentImpl {
    #[method]
    async fn mul(&self, arg: MyArg) -> Result<MyResp, RpcError> {
        Ok(MyResp { result: arg.value * 2 })
    }

    #[method]
    fn div(&self, arg: MyArg) -> Result<MyResp, RpcError> {
        Ok(MyResp { result: arg.value / 2 })
    }
}

pub struct MyServiceTraitImpl;

#[service]
impl MyService for MyServiceTraitImpl {
    async fn add(&self, arg: MyArg) -> Result<MyResp, RpcError> {
        Ok(MyResp { result: arg.value + 10 })
    }

    async fn sub(&self, arg: MyArg) -> Result<MyResp, RpcError> {
        Ok(MyResp { result: arg.value - 10 })
    }

    async fn always_error(&self, _arg: MyArg) -> Result<MyResp, RpcError> {
        Err(occams_rpc_core::error::RpcError::Text(
            "MyServiceTraitImpl always returns an error".to_string(),
        ))
    }
}

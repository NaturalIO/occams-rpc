use razor_rpc::client::ClientCaller;
use razor_rpc::client::task::APIClientReq;
use razor_rpc::{
    Codec,
    error::{RpcError, RpcIntErr},
};
use razor_rpc_codec::MsgpCodec;
use razor_rpc_macros::endpoint_async;
use razor_stream::client::ClientDefault;
use razor_stream::client::task::{
    ClientTaskAction, ClientTaskDecode, ClientTaskDone, ClientTaskEncode,
};
use razor_stream::proto::RpcAction;
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::sync::{Arc, Mutex};

// Struct to store request data for assertions
#[derive(Debug)]
struct StoredReq {
    action: String,
    req_msg: Vec<u8>,
}

// Mock Caller
struct MockCaller {
    stored_requests: Mutex<Vec<StoredReq>>,
}

impl MockCaller {
    fn new() -> Arc<Self> {
        Arc::new(Self { stored_requests: Mutex::new(Vec::new()) })
    }
}

impl ClientCaller for MockCaller {
    type Facts = ClientDefault<APIClientReq, orb_tokio::TokioRT, MsgpCodec>;

    async fn send_req(&self, mut task: APIClientReq) {
        println!("Sending request: {:?}", task);
        let codec = MsgpCodec::default();
        let action = match task.get_action() {
            RpcAction::Str(s) => s.to_string(),
            RpcAction::Num(_) => panic!("API client should not use Num action"),
        };

        let mut req_buf = Vec::new();
        task.encode_req(&codec, &mut req_buf).unwrap();

        self.stored_requests
            .lock()
            .unwrap()
            .push(StoredReq { action: action.clone(), req_msg: req_buf });

        let resp_buf = match action.as_str() {
            "MyTestService.add" => {
                let resp = AddResp { c: 30 };
                codec.encode(&resp).unwrap()
            }
            "MyTestService.no_args" => codec.encode(&()).unwrap(),
            "MyTestService.error_method" => {
                task.set_rpc_error(RpcIntErr::Method);
                task.done();
                return;
            }
            "MyFutureService.compute" => {
                let resp = ComputeResp { result: 42 };
                codec.encode(&resp).unwrap()
            }
            "NoAsyncTraitService.concat" => {
                let resp = ConcatResp { result: "HelloWorld".to_string() };
                codec.encode(&resp).unwrap()
            }
            _ => unreachable!(),
        };

        if action.as_str() != "MyTestService.error_method" {
            task.decode_resp(&codec, &resp_buf).unwrap();
            task.set_ok();
            task.done();
        }

        println!("Request completed");
    }
}

// Service Trait - Arguments and Response types
#[derive(Debug, Serialize, Deserialize, PartialEq, Clone)]
pub struct AddArgs {
    a: i32,
    b: i32,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Clone, Default)]
pub struct AddResp {
    c: i32,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Clone)]
pub struct ComputeArgs {
    x: i32,
    y: i32,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Clone, Default)]
pub struct ComputeResp {
    result: i32,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Clone)]
pub struct ConcatArgs {
    a: String,
    b: String,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Clone, Default)]
pub struct ConcatResp {
    result: String,
}

// Service Trait with async_trait
#[endpoint_async(MyTestClient)]
#[async_trait::async_trait]
pub trait MyTestService: Send + Sync + 'static {
    async fn add(&self, args: AddArgs) -> Result<AddResp, RpcError<()>>;
    async fn no_args(&self, _unused: ()) -> Result<(), RpcError<()>>;
    async fn error_method(&self, args: AddArgs) -> Result<AddResp, RpcError<()>>;
}

// Service Trait with impl Future (no async_trait)
#[endpoint_async(MyFutureClient)]
pub trait MyFutureService: Send + Sync + 'static {
    fn compute(
        &self, args: ComputeArgs,
    ) -> impl Future<Output = Result<ComputeResp, RpcError<()>>> + Send;
}

// Service Trait with impl Future but without async_trait attribute
#[endpoint_async(NoAsyncTraitClient)]
pub trait NoAsyncTraitService: Send + Sync + 'static {
    fn concat(
        &self, args: ConcatArgs,
    ) -> impl Future<Output = Result<ConcatResp, RpcError<()>>> + Send;
}

// Implementation for MyFutureService
impl MyFutureService for () {
    fn compute(
        &self, args: ComputeArgs,
    ) -> impl Future<Output = Result<ComputeResp, RpcError<()>>> + Send {
        async move { Ok(ComputeResp { result: args.x * args.y }) }
    }
}

// Implementation for NoAsyncTraitService
impl NoAsyncTraitService for () {
    fn concat(
        &self, args: ConcatArgs,
    ) -> impl Future<Output = Result<ConcatResp, RpcError<()>>> + Send {
        async move { Ok(ConcatResp { result: format!("{}{}", args.a, args.b) }) }
    }
}

// Test for async_trait service
#[tokio::test]
async fn test_endpoint_async_macro_with_async_trait() {
    let caller = MockCaller::new();
    let client = MyTestClient::new(caller.clone());

    // Call method with args
    let resp = client.add(AddArgs { a: 10, b: 20 }).await.unwrap();
    assert_eq!(resp, AddResp { c: 30 });
    {
        let requests = caller.stored_requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        let req1 = &requests[0];
        assert_eq!(req1.action, "MyTestService.add");

        let codec = MsgpCodec::default();
        let arg_val: AddArgs = codec.decode(&req1.req_msg).unwrap();
        assert_eq!(arg_val, AddArgs { a: 10, b: 20 });
    }

    // Call method with empty arg ()
    client.no_args(()).await.unwrap();
    {
        let requests = caller.stored_requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        let req2 = &requests[1];
        let codec = MsgpCodec::default();
        assert_eq!(req2.action, "MyTestService.no_args");
        let arg_val_2: () = codec.decode(&req2.req_msg).unwrap();
        assert_eq!(arg_val_2, ());
    }
}

// Test for impl Future service (no async_trait)
#[tokio::test]
async fn test_endpoint_async_macro_with_impl_future() {
    let caller = MockCaller::new();
    let client = MyFutureClient::new(caller.clone());

    // Call method with args that returns impl Future
    let future = client.compute(ComputeArgs { x: 5, y: 7 });
    let resp = future.await.unwrap();
    assert_eq!(resp, ComputeResp { result: 42 });
    {
        let requests = caller.stored_requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        let req1 = &requests[0];
        assert_eq!(req1.action, "MyFutureService.compute");

        let codec = MsgpCodec::default();
        let arg_val: ComputeArgs = codec.decode(&req1.req_msg).unwrap();
        assert_eq!(arg_val, ComputeArgs { x: 5, y: 7 });
    }
}

// Test for service without async_trait attribute but with impl Future
#[tokio::test]
async fn test_endpoint_async_macro_without_async_trait() {
    let caller = MockCaller::new();
    let client = NoAsyncTraitClient::new(caller.clone());

    // Call method with args that returns impl Future
    let future = client.concat(ConcatArgs { a: "Hello".to_string(), b: "World".to_string() });
    let resp = future.await.unwrap();
    assert_eq!(resp, ConcatResp { result: "HelloWorld".to_string() });
    {
        let requests = caller.stored_requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        let req1 = &requests[0];
        assert_eq!(req1.action, "NoAsyncTraitService.concat");

        let codec = MsgpCodec::default();
        let arg_val: ConcatArgs = codec.decode(&req1.req_msg).unwrap();
        assert_eq!(arg_val, ConcatArgs { a: "Hello".to_string(), b: "World".to_string() });
    }
}

// Test for error handling
#[tokio::test]
async fn test_endpoint_async_macro_with_error() {
    let caller = MockCaller::new();
    let client = MyTestClient::new(caller.clone());

    // Call method that returns an error
    let result = client.error_method(AddArgs { a: 1, b: 2 }).await;
    assert!(result.is_err());
    match result {
        Err(RpcError::Rpc(RpcIntErr::Method)) => {}
        _ => panic!("Expected RpcIntErr::Method error"),
    }
}

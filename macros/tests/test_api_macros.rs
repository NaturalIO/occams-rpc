use occams_rpc::server::ServiceTrait;
use occams_rpc_codec::MsgpCodec;
use occams_rpc_core::Codec;
use occams_rpc_stream::server::RespNoti;
use std::sync::Arc;

mod common;
use common::{
    create_mock_request, MyArg, MyAsyncTraitServiceImpl, MyResp, MyServiceImpl,
    MyServiceInherentImpl, MyServiceTraitImpl, MyServices,
};

#[tokio::test]
async fn test_service_macro() {
    let service_impl = MyServiceImpl;
    let codec = MsgpCodec::default();
    let (tx, rx) = crossfire::mpsc::unbounded_async();
    let noti = RespNoti::new(tx);
    let req = create_mock_request(
        1,
        "MyService".to_string(),
        "add".to_string(),
        &MyArg { value: 10 },
        noti.clone(),
    );

    ServiceTrait::serve(&service_impl, req).await;

    let resp = rx.recv().await.unwrap().unwrap();
    assert_eq!(resp.seq, 1);
    assert!(resp.res.is_ok());
    let decoded_resp: MyResp = codec.decode(&resp.msg.unwrap()).unwrap();
    assert_eq!(decoded_resp.result, 11);

    let req = create_mock_request(
        2,
        "MyService".to_string(),
        "sub".to_string(),
        &MyArg { value: 5 },
        noti.clone(),
    );

    ServiceTrait::serve(&service_impl, req).await;

    let resp = rx.recv().await.unwrap().unwrap();
    assert_eq!(resp.seq, 2);
    assert!(resp.res.is_ok());
    let decoded_resp: MyResp = codec.decode(&resp.msg.unwrap()).unwrap();
    assert_eq!(decoded_resp.result, 4);

    // Test unknown method
    let req = create_mock_request(
        3,
        "MyService".to_string(),
        "unknown".to_string(),
        &MyArg { value: 5 },
        noti.clone(),
    );

    ServiceTrait::serve(&service_impl, req).await;

    let resp = rx.recv().await.unwrap().unwrap();
    assert_eq!(resp.seq, 3);
    assert!(resp.res.is_err());

    // Test always_error method
    let req = create_mock_request(
        4,
        "MyService".to_string(),
        "always_error".to_string(),
        &MyArg { value: 100 },
        noti.clone(),
    );

    ServiceTrait::serve(&service_impl, req).await;

    let resp = rx.recv().await.unwrap().unwrap();
    assert_eq!(resp.seq, 4);
    assert!(resp.res.is_err());
    assert_eq!(
        resp.res.unwrap_err(),
        occams_rpc_core::error::RpcError::Text("This method always returns an error".to_string())
    );
}

#[tokio::test]
async fn test_service_enum_macro() {
    let codec = MsgpCodec::default();
    let (tx, rx) = crossfire::mpsc::unbounded_async();
    let noti = RespNoti::new(tx);
    let add_service = Arc::new(MyServiceImpl);
    let sub_service = Arc::new(MyServiceImpl);
    let services = MyServices::AddService(add_service.clone());
    // Test 'add' method through enum
    let req = create_mock_request(
        1,
        "MyService".to_string(),
        "add".to_string(),
        &MyArg { value: 10 },
        noti.clone(),
    );

    ServiceTrait::serve(&services, req).await;

    let resp = rx.recv().await.unwrap().unwrap();
    assert_eq!(resp.seq, 1);
    assert!(resp.res.is_ok());
    let decoded_resp: MyResp = codec.decode(&resp.msg.unwrap()).unwrap();
    assert_eq!(decoded_resp.result, 11);

    // Change the enum variant
    let services = MyServices::SubService(sub_service.clone());

    // Test 'sub' method through enum
    let req = create_mock_request(
        2,
        "MyService".to_string(),
        "sub".to_string(),
        &MyArg { value: 5 },
        noti.clone(),
    );

    ServiceTrait::serve(&services, req).await;

    let resp = rx.recv().await.unwrap().unwrap();
    assert_eq!(resp.seq, 2);
    assert!(resp.res.is_ok());
    let decoded_resp: MyResp = codec.decode(&resp.msg.unwrap()).unwrap();
    assert_eq!(decoded_resp.result, 4);
}

#[tokio::test]
async fn test_async_trait_service_macro() {
    let codec = MsgpCodec::default();
    let (tx, rx) = crossfire::mpsc::unbounded_async();
    let noti = RespNoti::new(tx);
    let service_impl = Arc::new(MyAsyncTraitServiceImpl);

    // Test 'mul' method (async)
    let req = create_mock_request(
        1,
        "MyAsyncTraitService".to_string(),
        "mul".to_string(),
        &MyArg { value: 10 },
        noti.clone(),
    );

    ServiceTrait::serve(&service_impl, req).await;

    let resp = rx.recv().await.unwrap().unwrap();
    assert_eq!(resp.seq, 1);
    assert!(resp.res.is_ok());
    let decoded_resp: MyResp = codec.decode(&resp.msg.unwrap()).unwrap();
    assert_eq!(decoded_resp.result, 20);

    // Test 'div' method (non-async)
    let req = create_mock_request(
        2,
        "MyAsyncTraitService".to_string(),
        "div".to_string(),
        &MyArg { value: 10 },
        noti.clone(),
    );

    ServiceTrait::serve(&service_impl, req).await;

    let resp = rx.recv().await.unwrap().unwrap();
    assert_eq!(resp.seq, 2);
    assert!(resp.res.is_ok());
    let decoded_resp: MyResp = codec.decode(&resp.msg.unwrap()).unwrap();
    assert_eq!(decoded_resp.result, 5);
}

#[tokio::test]
async fn test_service_inherent_impl_macro() {
    let codec = MsgpCodec::default();
    let (tx, rx) = crossfire::mpsc::unbounded_async();
    let noti = RespNoti::new(tx);
    let service_impl = MyServiceInherentImpl;

    // Test 'mul' method (async)
    let req = create_mock_request(
        1,
        "MyServiceInherentImpl".to_string(),
        "mul".to_string(),
        &MyArg { value: 10 },
        noti.clone(),
    );

    ServiceTrait::serve(&service_impl, req).await;

    let resp = rx.recv().await.unwrap().unwrap();
    assert_eq!(resp.seq, 1);
    assert!(resp.res.is_ok());
    let decoded_resp: MyResp = codec.decode(&resp.msg.unwrap()).unwrap();
    assert_eq!(decoded_resp.result, 20);

    // Test 'div' method (non-async)
    let req = create_mock_request(
        2,
        "MyServiceInherentImpl".to_string(),
        "div".to_string(),
        &MyArg { value: 10 },
        noti.clone(),
    );

    ServiceTrait::serve(&service_impl, req).await;

    let resp = rx.recv().await.unwrap().unwrap();
    assert_eq!(resp.seq, 2);
    assert!(resp.res.is_ok());
    let decoded_resp: MyResp = codec.decode(&resp.msg.unwrap()).unwrap();
    assert_eq!(decoded_resp.result, 5);
}

#[tokio::test]
async fn test_service_trait_impl_macro() {
    let codec = MsgpCodec::default();
    let (tx, rx) = crossfire::mpsc::unbounded_async();
    let noti = RespNoti::new(tx);
    let service_impl = MyServiceTraitImpl;

    // Test 'add' method
    let req = create_mock_request(
        1,
        "MyService".to_string(),
        "add".to_string(),
        &MyArg { value: 10 },
        noti.clone(),
    );

    ServiceTrait::serve(&service_impl, req).await;

    let resp = rx.recv().await.unwrap().unwrap();
    assert_eq!(resp.seq, 1);
    assert!(resp.res.is_ok());
    let decoded_resp: MyResp = codec.decode(&resp.msg.unwrap()).unwrap();
    assert_eq!(decoded_resp.result, 20); // 10 + 10

    // Test 'sub' method
    let req = create_mock_request(
        2,
        "MyService".to_string(),
        "sub".to_string(),
        &MyArg { value: 10 },
        noti.clone(),
    );

    ServiceTrait::serve(&service_impl, req).await;

    let resp = rx.recv().await.unwrap().unwrap();
    assert_eq!(resp.seq, 2);
    assert!(resp.res.is_ok());
    let decoded_resp: MyResp = codec.decode(&resp.msg.unwrap()).unwrap();
    assert_eq!(decoded_resp.result, 0); // 10 - 10

    // Test always_error method
    let req = create_mock_request(
        3,
        "MyService".to_string(),
        "always_error".to_string(),
        &MyArg { value: 100 },
        noti.clone(),
    );

    ServiceTrait::serve(&service_impl, req).await;

    let resp = rx.recv().await.unwrap().unwrap();
    assert_eq!(resp.seq, 3);
    assert!(resp.res.is_err());
    assert_eq!(
        resp.res.unwrap_err(),
        occams_rpc_core::error::RpcError::Text(
            "MyServiceTraitImpl always returns an error".to_string()
        )
    );
}

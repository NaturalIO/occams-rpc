use crossfire::{MTx, mpsc};
use occams_rpc_codec::MsgpCodec;
use occams_rpc_core::error::*;
use occams_rpc_stream::{
    client::{
        ClientTaskAction, ClientTaskCommon, ClientTaskDecode, ClientTaskDone, ClientTaskEncode,
    },
    proto::RpcAction,
};
use occams_rpc_stream_macros::{client_task, client_task_enum};

#[client_task(1, debug)]
struct TaskA {
    #[field(common)]
    common: ClientTaskCommon,
    #[field(req)]
    req: String,
    #[field(resp)]
    resp: Option<String>,
    #[field(res)]
    res: Option<Result<(), ServerErr>>,
    #[field(noti)]
    noti: Option<MTx<MyTask>>,
}

#[client_task("task_b", debug)]
struct TaskB {
    #[field(common)]
    common: ClientTaskCommon,
    #[field(req)]
    req: u32,
    #[field(resp)]
    resp: Option<u32>,
    #[field(res)]
    res: Option<Result<(), ServerErr>>,
    #[field(noti)]
    noti: Option<MTx<MyTask>>,
}

#[client_task(3, debug)]
struct TaskC {
    #[field(common)]
    common: ClientTaskCommon,
    #[field(req)]
    req: (),
    #[field(resp)]
    resp: Option<()>,
    #[field(req_blob)]
    req_blob: Vec<u8>,
    #[field(resp_blob)]
    resp_blob: Option<Vec<u8>>,
    #[field(res)]
    res: Option<Result<(), ServerErr>>,
    #[field(noti)]
    noti: Option<MTx<MyTask>>,
}

#[client_task_enum]
#[derive(Debug)]
enum MyTask {
    A(TaskA),
    B(TaskB),
    C(TaskC),
}

#[test]
fn test_client_task_enum_delegation() {
    let (tx, rx) = mpsc::unbounded_blocking();
    let task_a = TaskA {
        common: ClientTaskCommon::default(),
        req: "hello".to_string(),
        resp: None,
        res: None,
        noti: Some(tx.clone()),
    };

    let task_b = TaskB {
        common: ClientTaskCommon::default(),
        req: 123,
        resp: None,
        res: None,
        noti: Some(tx.clone()),
    };

    let task_c = TaskC {
        common: ClientTaskCommon::default(),
        req: (),
        resp: None,
        req_blob: vec![1, 2, 3],
        resp_blob: Some(vec![]),
        res: None,
        noti: Some(tx.clone()),
    };

    // Test From impls
    let mut enum_task_a: MyTask = task_a.into();

    // Test Deref
    enum_task_a.set_seq(10);

    // Test ClientTaskAction delegation
    assert_eq!(enum_task_a.get_action(), RpcAction::Num(1));

    let mut enum_task_b: MyTask = task_b.into();
    assert_eq!(enum_task_b.get_action(), RpcAction::Str("task_b"));
    let mut enum_task_c: MyTask = task_c.into();
    assert_eq!(enum_task_c.get_action(), RpcAction::Num(3));

    // Test ClientTask delegation
    assert_eq!(enum_task_a.get_result(), Err(&RPC_ERR_INTERNAL));
    enum_task_a.set_result(Ok(()));
    let received = rx.recv().unwrap();
    assert!(matches!(received, MyTask::A(_)));
    assert_eq!(received.get_result(), Ok(()));

    // Test ClientTaskEncode/Decode delegation
    let codec = MsgpCodec::default();
    let msg_buf = enum_task_b.encode_req(&codec).expect("encode_req ok");

    if let MyTask::B(t) = &mut enum_task_b {
        t.resp = None;
    }
    assert!(enum_task_b.decode_resp(&codec, &msg_buf).is_ok());
    if let MyTask::B(t) = enum_task_b {
        assert!(t.resp.is_some());
    } else {
        panic!("Wrong enum variant");
    }

    // Test blob delegation
    assert_eq!(enum_task_c.get_req_blob(), Some(vec![1, 2, 3].as_slice()));
    assert!(enum_task_c.reserve_resp_blob(10).is_some());
}

#[client_task(999, debug)] // Dummy action
struct TaskActionOverwrite {
    #[field(common)]
    common: ClientTaskCommon,
    #[field(req)]
    req: String,
    #[field(resp)]
    resp: Option<String>,
    #[field(res)]
    res: Option<Result<(), ServerErr>>,
    #[field(noti)]
    noti: Option<MTx<MyTaskWithAction>>,
}

#[client_task(999, debug)] // Dummy action
struct TaskActionDelegate {
    #[field(common)]
    common: ClientTaskCommon,
    #[field(req)]
    req: String,
    #[field(resp)]
    resp: Option<String>,
    #[field(res)]
    res: Option<Result<(), ServerErr>>,
    #[field(noti)]
    noti: Option<MTx<MyTaskWithAction>>,
}

#[client_task("task_b", debug)]
struct TaskBWithAction {
    #[field(common)]
    common: ClientTaskCommon,
    #[field(req)]
    req: u32,
    #[field(resp)]
    resp: Option<u32>,
    #[field(res)]
    res: Option<Result<(), ServerErr>>,
    #[field(noti)]
    noti: Option<MTx<MyTaskWithAction>>,
}

#[client_task_enum]
#[derive(Debug)]
enum MyTaskWithAction {
    #[action(100)]
    A(TaskActionOverwrite),
    #[action("action_b")]
    B(TaskBWithAction),
    C(TaskActionDelegate),
}

#[test]
fn test_client_task_enum_with_action_attribute() {
    let (tx, _rx) = mpsc::unbounded_blocking();
    let task_a = TaskActionOverwrite {
        common: ClientTaskCommon::default(),
        req: "hello".to_string(),
        resp: None,
        res: None,
        noti: Some(tx.clone()),
    };

    let task_b = TaskBWithAction {
        common: ClientTaskCommon::default(),
        req: 123,
        resp: None,
        res: None,
        noti: Some(tx.clone()),
    };

    let enum_task_a: MyTaskWithAction = task_a.into();
    assert_eq!(enum_task_a.get_action(), RpcAction::Num(100));

    let enum_task_b: MyTaskWithAction = task_b.into();
    assert_eq!(enum_task_b.get_action(), RpcAction::Str("action_b"));

    let task_c = TaskActionDelegate {
        common: ClientTaskCommon::default(),
        req: "hello".to_string(),
        resp: None,
        res: None,
        noti: Some(tx.clone()),
    };

    let enum_task_c: MyTaskWithAction = task_c.into();
    assert_eq!(enum_task_c.get_action(), RpcAction::Num(999));
}

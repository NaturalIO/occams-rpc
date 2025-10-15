# `server_task_enum` Macro

## Overview

The `#[server_task_enum]` is a procedural attribute macro designed to streamline the creation of server-side task enums in `occams-rpc`. It automates the implementation of several key traits required for processing RPC requests, reducing boilerplate and improving code maintainability.

When applied to an enum, this macro generates implementations for `ServerTaskDecode`, `ServerTaskEncode`, and `ServerTaskDone`, as well as `From` implementations for each variant's inner type.

## Usage

```rust
use occams_rpc::stream::server::{ServerTaskDecode, ServerTaskEncode, ServerTaskDone};
use occams_rpc_macros::server_task_enum;

// Define your subtypes for each task
struct SubTask1;
struct SubTask2;

// Implement the required traits for your subtypes
impl<'a, T: Send + 'static> ServerTaskDecode<'a, T> for SubTask1 { /* ... */ }
impl ServerTaskEncode for SubTask1 { /* ... */ }
impl<T: RpcServerTaskResp> ServerTaskDone<T> for SubTask1 { /* ... */ }

impl<'a, T: Send + 'static> ServerTaskDecode<'a, T> for SubTask2 { /* ... */ }
impl ServerTaskEncode for SubTask2 { /* ... */ }
impl<T: RpcServerTaskResp> ServerTaskDone<T> for SubTask2 { /* ... */ }


The `server_task_enum` can accept either or both of "req" and "resp" flags.
- If "resp" is specified, the response type for `ServerTaskDecode<R>` and `ServerTaskDone<R>` is implicitly `Self` (the enum itself). In this case, `resp_type` can be omitted.

- If only "req" is specified (and "resp" is not), then `resp_type` must be provided. This `resp_type` specifies the type `<R>` for parameters of `ServerTaskDecode<R>` and `ServerTaskDone<R>`.
- If "req" is specified, each variant must also have an `#[action]` attribute to associate it with an RPC action.

#[server_task_enum(req, resp)] // Example with both req and resp, resp_type omitted
#[derive(Debug)]
pub enum ServerTask {
    #[action(1)]
    Task1(SubTask1),

    #[action("sub_task_2")]
    Task2(SubTask2),
}

#[server_task_enum(req, resp_type = RpcResp)] // Example with only req, resp_type is required
#[derive(Debug)]
pub enum ServerTask {
    #[action(1)]
    Task1(SubTask1),
}

#[server_task_enum(resp)] // Example with only resp, resp_type and action are not required
#[derive(Debug)]
pub enum ServerTask {
    Task1(SubTask1),
}
```

The `#[action()]` can contain multiple items, separated by a comma. It will then call SubType::decode_req in multiple situations.
In this case, the SubType is required to implement the `ServerTaskAction` trait to return the actual action it stored.

```rust
#[server_task_enum(req, resp_type = RpcResp)] // Example with only req, resp_type is required
#[derive(Debug)]
pub enum ServerTask {
    #[action(1, 2, 3)]
    Task1(SubTask1),
}

pub struct SubTask1 {
    action_field: i32,
    ...
}

When an `#[action()]` item is not numeric or a string, it can be one or more Enum values. In this case, the value is converted to a number when parsing.

``` rust
#[server_task_enum(req, resp_type = RpcResp)] // Example with only req, resp_type is required
#[derive(Debug)]
pub enum ServerTask {
    #[action(Action::Read, Action::Write)]
    Task1(SubTask1),
}

#[repr(u8)]
pub enum Action {
    Read=1,
    Write=2,
    Seek=3,
}
```

## Generated Code

The macro generates the following implementations for the enum:

### `From<SubType> for Enum`

For each variant, a `From` implementation is generated to allow easy conversion from a subtype to the enum.

```rust
impl From<SubTask1> for ServerTask {
    fn from(task: SubTask1) -> Self {
        ServerTask::Task1(task)
    }
}
```

The macro should forbid potential duplicate types inside the variants.

## Requirements

- The macro must be applied to an enum.
- Each variant of the enum must be a tuple-style variant with a single field.
- Each variant must have an `#[action]` attribute, which can be a numeric or string literal (e.g., `#[action(1)]` or `#[action("my_action")]`).
- The inner type of each variant must implement `ServerTaskDecode`, `ServerTaskEncode`, and `ServerTaskDone`.

### `ServerTaskDecode`

If `req` is specified with `server_task_enum` attribute, the macro generates an implementation of `ServerTaskDecode` that decodes an incoming request based on the `RpcAction`. It matches the action from the request with the `#[action]` attribute of the variants and delegates the decoding to the corresponding subtype's `decode_req` method.

### `ServerTaskEncode`

If `resp` is specified with `server_task_enum` attribute, an implementation of `ServerTaskEncode` is generated to encode the response. It matches on the enum variant and delegates the encoding to the inner subtype's `encode_resp` method.

### `ServerTaskDone`

If `resp` is specified with `server_task_enum` attribute, an implementation of `ServerTaskDone` is generated to handle the completion of a task. It delegates the `set_result` call to the inner subtype.

### `ServerTaskAction::get_action()`

A `get_action(&self) -> RpcAction` method is generated for the enum, which returns the `RpcAction` associated with the current variant. This method is part of the `ServerTaskAction` trait.


## Trait Delegation and Compile-Time Checks

The macro relies on a "delegation" pattern. It generates code that calls the trait methods on the inner subtypes. This means that if a subtype does not implement a required trait (e.g., `ServerTaskEncode`), the code will fail to compile with a clear error message about the missing trait implementation.

This approach ensures type safety and correctness at compile time, leveraging the Rust compiler to enforce the necessary constraints on the task subtypes.

# The server-side macros for API interface

## Requirement

- Provide similar functionality to GRPC
- All service method should have one arg `A` and one response of `Result<T, RpcError<E>>`, where `E` is a user-defined error type that implements `RpcErrCodec`. `A` and `T` may be empty structs.

## Trait

There's a `ServiceTrait` which defines a `serve` function, which accept request

```rust
struct Request<C: Codec> {
    seq: u64,
    service: String,
    method: String,
    req: Option<Vec<u8>>,
    codec: Arc<C>,
    noti: RespNoti<Response>,
}
```

The `ServiceTrait::serve(req)` should:
    - decode the request from bytes to request struct
    - call the method in itself, to get a response
    - set_result or set_error to the Request, and encode a Response contains message bytes or an error
    - send the Response through RespNoti

## macros

### service `#[service]`

The `#[service]` macro is applied to an `impl` block to automatically generate the `ServiceTrait` implementation for the type.

-   When applied to an inherent `impl` block, methods intended as service methods should be marked with `#[method]`.
-   When applied to a trait `impl` block, all methods defined in the trait will be registered as service methods (no `#[method]` marker needed).
-   The macro can handle methods with different user-defined error types (e.g. `String`, `i32`, `nix::errno::Errno`) in the same `impl` block.

The service method recognizes:
- `async fn`
- `impl Future`
- trait methods wrapped by `async_trait`

The macro will iterate all methods and generate a `ServiceTrait::serve()` implementation.

#### Example: Inherent `impl`

```rust
use occams_rpc_api_macros::{method, service};
use occams_rpc_core::error::RpcError;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
pub struct MyArg {
    pub value: u32,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct MyResp {
    pub result: u32,
}

#[derive(Clone)]
pub struct MyServiceInherentImpl;

#[service]
impl MyServiceInherentImpl {
    #[method]
    async fn mul(&self, arg: MyArg) -> Result<MyResp, RpcError<String>> {
        Ok(MyResp { result: arg.value * 2 })
    }

    #[method]
    async fn div(&self, arg: MyArg) -> Result<MyResp, RpcError<i32>> {
        if arg.value == 0 {
            return Err(1.into()); // Return an i32 error
        }
        Ok(MyResp { result: arg.value / 2 })
    }
}
```

#### Example: Trait `impl`

```rust
use occams_rpc_api_macros::service;
use occams_rpc_core::error::RpcError;
use serde::{Deserialize, Serialize};
use std::future::Future;

// Assuming MyArg and MyResp are defined as above

pub trait MyService {
    fn add(&self, arg: MyArg) -> impl Future<Output = Result<MyResp, RpcError<String>>> + Send;
    fn sub(&self, arg: MyArg) -> impl Future<Output = Result<MyResp, RpcError<String>>> + Send;
}

#[derive(Clone)]
pub struct MyServiceTraitImpl;

#[service]
impl MyService for MyServiceTraitImpl {
    async fn add(&self, arg: MyArg) -> Result<MyResp, RpcError<String>> {
        Ok(MyResp { result: arg.value + 10 })
    }

    async fn sub(&self, arg: MyArg) -> Result<MyResp, RpcError<String>> {
        Ok(MyResp { result: arg.value - 10 })
    }
}
```

### Service Dispatcher `#[service_mux_struct]`

The `#[service_mux_struct]` macro is applied to a **struct** to implement `ServiceTrait` on it. It acts as a dispatcher, routing `serve()` calls to the correct service based on the `req.service` field.

Each field in the struct must hold a service that implements `ServiceTrait` (e.g., wrapped in an `Arc`). The macro generates a `serve` implementation that matches `req.service` against the field names of the struct.

#### Example: Service Dispatcher Struct

```rust
use occams_rpc_api_macros::{service, service_mux_struct, method};
use occams_rpc_core::error::RpcError;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

// Define some request/response types
#[derive(Debug, Deserialize, Serialize)]
pub struct MyArg { pub value: u32, }
#[derive(Debug, Deserialize, Serialize)]
pub struct MyResp { pub result: u32, }

// Define a service
pub struct MyFirstService;
#[service]
impl MyFirstService {
    #[method]
    async fn add(&self, arg: MyArg) -> Result<MyResp, RpcError<String>> {
        Ok(MyResp { result: arg.value + 1 })
    }
}

// Define another service
pub struct MySecondService;
#[service]
impl MySecondService {
    #[method]
    async fn sub(&self, arg: MyArg) -> Result<MyResp, RpcError<String>> {
        Ok(MyResp { result: arg.value - 1 })
    }
}

// Use the dispatcher to combine them
#[service_mux_struct]
pub struct MyServiceDispatcher {
    my_first: Arc<MyFirstService>,
    my_second: Arc<MySecondService>,
}

// The generated `serve` will look something like this:
//
// impl<C: Codec> ServiceTrait<C> for MyServiceDispatcher {
//     async fn serve(&self, req: Request<C>) -> ... {
//         match req.service.as_str() {
//             "my_first" => self.my_first.serve(req).await,
//             "my_second" => self.my_second.serve(req).await,
//             _ => req.set_rpc_error(RpcIntErr::Service),
//         }
//     }
// }
```

# The server-side macros for API interface

## Requirement

- Provide similar functionality to GRPC
- All service method should have one arg `A` and one response of `Result<T, RpcError>`. `A` and `T`
  may be empty struct.

## Trait

There's a `ServiceTrait` which defines a `serve` function, which accept request

```
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
    - set_result or set_error to the Request, and encode a Response contains message bytes or RpcError
    - send the Response through RespNoti

## macros

### service `#[service]`

The `#[service]` macro is applied to an `impl` block to automatically generate the `ServiceTrait` implementation for the type.

-   When applied to an inherent `impl` block, methods intended as service methods should be marked with `#[method]`.
-   When applied to a trait `impl` block, all methods defined in the trait will be registered as service methods (no `#[method]` marker needed).

The service method recognizes:
- `fn` (which consider non-block)
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
    async fn mul(&self, arg: MyArg) -> Result<MyResp, RpcError> {
        Ok(MyResp { result: arg.value * 2 })
    }

    #[method]
    fn div(&self, arg: MyArg) -> Result<MyResp, RpcError> {
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
    fn add(&self, arg: MyArg) -> impl Future<Output = Result<MyResp, RpcError>> + Send;
    fn sub(&self, arg: MyArg) -> impl Future<Output = Result<MyResp, RpcError>> + Send;
}

#[derive(Clone)]
pub struct MyServiceTraitImpl;

#[service]
impl MyService for MyServiceTraitImpl {
    async fn add(&self, arg: MyArg) -> Result<MyResp, RpcError> {
        Ok(MyResp { result: arg.value + 10 })
    }

    async fn sub(&self, arg: MyArg) -> Result<MyResp, RpcError> {
        Ok(MyResp { result: arg.value - 10 })
    }
}
```

### service multiplexer `#[service_enum]`

The `#[service_enum]` macro is applied to an enum to implement `ServiceTrait` on it, which dispatches `serve()` calls to its variants. Each variant must be a newtype variant holding a service that implements `ServiceTrait`.

#### Example: Service Enum

```rust
use occams_rpc_api_macros::{service, service_enum};
use occams_rpc_core::error::RpcError;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

// Assuming MyArg and MyResp are defined as above

#[derive(Clone)]
pub struct MyServiceImpl;

pub trait MyService {
    fn add(&self, arg: MyArg) -> impl std::future::Future<Output = Result<MyResp, RpcError>> + Send;
    fn sub(&self, arg: MyArg) -> impl std::future::Future<Output = Result<MyResp, RpcError>> + Send;
}

#[service]
impl MyService for MyServiceImpl {
    async fn add(&self, arg: MyArg) -> Result<MyResp, RpcError> {
        Ok(MyResp { result: arg.value + 1 })
    }

    async fn sub(&self, arg: MyArg) -> Result<MyResp, RpcError> {
        Ok(MyResp { result: arg.value - 1 })
    }
}

#[service_enum]
pub enum MyServices {
    AddService(Arc<MyServiceImpl>),
    SubService(Arc<MyServiceImpl>),
}
```


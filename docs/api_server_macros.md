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

When Apply #[service] on a struct, service method should be mark as #[method].
When Apply #[service(SomeTrait)] on a struct, will register service method defined by SomeTrait.

the service method regconizes:
- `fn` (which consider non-block)
- `async fn`
- `impl Future`
- trait methods wrapped by `async_trait`

the macro will iterate all methods and generate a ServiceTrait::serve() implement.

### service multiplexer `#[service_enum]`

impl a ServiceTrait on a enum, which dispatch serve() to it's variants


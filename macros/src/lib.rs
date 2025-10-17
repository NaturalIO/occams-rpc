extern crate proc_macro;
use proc_macro::TokenStream;

mod service;

mod service_mux_struct;

/// The `#[service]` macro is applied to an `impl` block to automatically generate the `ServiceTrait` implementation for the type.
///
/// - When applied to an inherent `impl` block, methods intended as service methods should be marked with `#[method]`.
/// - When applied to a trait `impl` block, all methods defined in the trait will be registered as service methods.
/// - All service methods must return `Result<T, RpcError<E>>`, where `E` is a user-defined error type that implements `RpcErrCodec`.
///
/// The service method recognizes:
/// - `fn` (which is considered non-blocking)
/// - `async fn`
/// - `impl Future`
/// - trait methods wrapped by `async_trait`
#[proc_macro_attribute]
pub fn service(_attr: TokenStream, item: TokenStream) -> TokenStream {
    service::service(_attr, item)
}

/// A marker attribute for methods in an inherent `impl` block that should be exposed as RPC methods.
/// This is not needed when using a trait-based implementation.
#[proc_macro_attribute]
pub fn method(_attr: TokenStream, item: TokenStream) -> TokenStream {
    item
}

/// The `#[service_mux_struct]` macro is applied to a **struct** to implement `ServiceTrait` on it.
/// It acts as a dispatcher, routing `serve()` calls to the correct service based on the `req.service` field.
///
/// Each field in the struct must hold a service that implements `ServiceTrait` (e.g., wrapped in an `Arc`).
/// The macro generates a `serve` implementation that matches `req.service` against the field names of the struct.
#[proc_macro_attribute]
pub fn service_mux_struct(_attr: TokenStream, item: TokenStream) -> TokenStream {
    service_mux_struct::service_mux_struct(_attr, item)
}

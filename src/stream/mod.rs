// low level of rpc

pub mod client;
pub mod client_task;
mod notifier;
pub mod proto;
pub mod server;
mod throttler;

#[derive(Debug, Clone, Copy)]
pub enum RpcAction<'a> {
    Str(&'a str),
    Num(i32),
}

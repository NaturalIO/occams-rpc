use serde::{Deserialize, Serialize};

/// Interface for [occams-rpc-codec](https://docs.rs/occams-rpc-codec)
///
/// The codec is immutable, if need changing (like setting up cipher), should have inner
/// mutablilty
pub trait Codec: Default + Send + Sync + Sized + 'static {
    fn encode<T: Serialize>(&self, task: &T) -> Result<Vec<u8>, ()>;

    fn decode<'a, T: Deserialize<'a>>(&self, buf: &'a [u8]) -> Result<T, ()>;
}

#[allow(dead_code)]
pub trait BytesCodec: Codec {
    fn encode<T: Serialize>(&self, task: &T) -> Result<Vec<u8>, ()>
    where
        T: AsRef<u8>;

    fn decode<'a, T: Deserialize<'a>>(&self, buf: &'a [u8]) -> Result<T, ()>
    where
        T: AsRef<u8>;
}

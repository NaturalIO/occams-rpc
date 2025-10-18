use serde::{Deserialize, Serialize};

/*
 *  Note that there's no unify output interface in each serde impl,
 *  whatever we want to serialize into (std::io::Write / Buffer/ Vec<u8>),
 *  require the codec implment to match.
 */

/// Interface for [occams-rpc-codec](https://docs.rs/occams-rpc-codec)
///
/// The codec is immutable, if need changing (like setting up cipher), should have inner
/// mutablilty
pub trait Codec: Default + Send + Sync + Sized + 'static {
    fn encode<T: Serialize>(&self, task: &T) -> Result<Vec<u8>, ()>;

    /// sererialized the msg into buf (with std::io::Writer), and return the size written
    fn encode_into<T: Serialize>(&self, task: &T, buf: &mut Vec<u8>) -> Result<usize, ()>;

    fn decode<'a, T: Deserialize<'a>>(&self, buf: &'a [u8]) -> Result<T, ()>;
}

/*
#[allow(dead_code)]
pub trait BytesCodec: Codec {

    fn encode<T: Serialize>(&self, task: &T) -> Result<Vec<u8>, ()> where T: AsRef<u8>;

    fn decode<'a, T: Deserialize<'a>>(&self, buf: &'a [u8]) -> Result<T, ()> where T: AsRef<u8>;
}
*/

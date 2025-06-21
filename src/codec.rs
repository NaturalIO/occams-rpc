use serde::{Deserialize, Serialize};
use std::fmt::Display;

pub trait Codec {
    fn encode<T: Serialize + Display>(task: &T) -> Result<Vec<u8>, ()>;

    fn decode<'a, T: Deserialize<'a>>(buf: &'a [u8]) -> Result<T, ()>;
}

pub struct MsgpCodec();

impl Codec for MsgpCodec {
    #[inline(always)]
    fn encode<T: Serialize + Display>(task: &T) -> Result<Vec<u8>, ()> {
        match rmp_serde::encode::to_vec_named(task) {
            Ok(buf) => return Ok(buf),
            Err(e) => {
                error!("{} encode error: {:?}", task, e);
                return Err(());
            }
        }
    }

    #[inline(always)]
    fn decode<'a, T: Deserialize<'a>>(buf: &'a [u8]) -> Result<T, ()> {
        match rmp_serde::decode::from_slice::<T>(buf) {
            Err(e) => {
                warn!("decode error: {:?}", e);
                return Err(());
            }
            Ok(s) => return Ok(s),
        }
    }
}

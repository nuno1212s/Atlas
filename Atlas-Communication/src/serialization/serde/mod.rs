use crate::serialization::Serializable;
use anyhow::Context;
use std::io::{Read, Write};

pub fn serialize_message<W, T>(m: &T::Message, w: &mut W) -> atlas_common::error::Result<()>
where
    W: Write + AsMut<[u8]>,
    T: Serializable,
{
    //TODO: Could this be faster if it were returning the vec and allowing bincode to manage it?
    bincode::serde::encode_into_std_write(m, w, bincode::config::standard()).context(format!(
        "Failed to serialize message {} bytes len",
        w.as_mut().len()
    ))?;

    Ok(())
}

pub fn deserialize_message<R, T>(r: R) -> atlas_common::error::Result<T::Message>
where
    T: Serializable,
    R: Read + AsRef<[u8]>,
{
    let (msg, _read_bytes) =
        bincode::serde::decode_from_slice(r.as_ref(), bincode::config::standard())
            .context("Failed to deserialize message")?;

    Ok(msg)
}

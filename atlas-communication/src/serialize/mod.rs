//! This module is responsible for serializing wire messages in `febft`.
//!
//! All relevant types transmitted over the wire are `serde` aware, if
//! this feature is enabled with `serialize_serde`. Slightly more exotic
//! serialization routines, for better throughput, can be utilized, such
//! as [Cap'n'Proto](https://capnproto.org/capnp-tool.html), but these are
//! expected to be implemented by the user.

use std::io::{Read, Write};
use std::sync::Arc;
use bytes::Bytes;
use atlas_common::error::*;

#[cfg(feature = "serialize_serde")]
use ::serde::{Deserialize, Serialize};

use atlas_common::crypto::hash::{Context, Digest};
use crate::message::{Header,  NetworkMessageKind};
use crate::message_signing::NetworkMessageSignatureVerifier;
use crate::reconfiguration_node::NetworkInformationProvider;

#[cfg(feature = "serialize_capnp")]
pub mod capnp;

#[cfg(feature = "serialize_serde")]
pub mod serde;

/// The buffer type used to serialize messages into.
pub type Buf = Bytes;

pub fn serialize_message<W, RM, PM>(w: &mut W, msg: &NetworkMessageKind<RM, PM>) -> Result<()>
    where W: Write + AsRef<[u8]> + AsMut<[u8]>, RM: Serializable, PM: Serializable {
    #[cfg(feature = "serialize_capnp")]
    capnp::serialize_message::<W, RM, PM>(w, msg)?;

    #[cfg(feature = "serialize_serde")]
    serde::serialize_message::<W, RM, PM>(msg, w)?;

    Ok(())
}

pub fn deserialize_message<R, RM, PM>(r: R) -> Result<NetworkMessageKind<RM, PM>>
    where R: Read + AsRef<[u8]>, RM: Serializable + 'static, PM: Serializable + 'static {
    #[cfg(feature = "serialize_capnp")]
        let result = capnp::deserialize_message::<R, RM, PM>(r)?;

    #[cfg(feature = "serialize_serde")]
        let result = serde::deserialize_message::<R, RM, PM>(r)?;

    Ok(result)
}

pub fn serialize_digest<W, RM, PM>(message: &NetworkMessageKind<RM, PM>, w: &mut W) -> Result<Digest>
    where W: Write + AsRef<[u8]> + AsMut<[u8]>, RM: Serializable, PM: Serializable {
    serialize_message::<W, RM, PM>(w, message)?;

    let mut ctx = Context::new();
    ctx.update(w.as_ref());
    Ok(ctx.finish())
}


/// The trait that should be implemented for all systems which wish to use this communication method
pub trait Serializable: Send + Sync {
    #[cfg(feature = "serialize_capnp")]
    type Message: Send + Clone;

    #[cfg(feature = "serialize_serde")]
    type Message: for<'a> Deserialize<'a> + Serialize + Send + Clone;

    /// Verify the signature of the internal message structure, to make sure all makes sense
    ///
    fn verify_message_internal<SV, NI>(info_provider: &Arc<NI>, header: &Header, msg: &Self::Message, full_raw_msg: &Buf) -> Result<bool> where
        NI: NetworkInformationProvider,
        SV: NetworkMessageSignatureVerifier<Self, NI>, Self: Sized;

    #[cfg(feature = "serialize_capnp")]
    fn serialize_capnp(builder: febft_capnp::messages_capnp::system::Builder, msg: &Self::Message) -> Result<()>;

    #[cfg(feature = "serialize_capnp")]
    fn deserialize_capnp(reader: febft_capnp::messages_capnp::system::Reader) -> Result<Self::Message>;
}


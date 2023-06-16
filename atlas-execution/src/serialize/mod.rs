use std::io::{Read, Write};
use std::mem::size_of;
#[cfg(feature = "serialize_serde")]
use serde::{Deserialize, Serialize};
use atlas_common::crypto::hash::{Context, Digest};

use atlas_common::error::*;

pub trait ApplicationData {

    /// Represents the requests forwarded to replicas by the
    /// clients of the BFT system.
    #[cfg(feature = "serialize_serde")]
    type Request: for<'a> Deserialize<'a> + Serialize + Send + Clone;

    #[cfg(feature = "serialize_capnp")]
    type Request: Send + Clone;

    /// Represents the replies forwarded to clients by replicas
    /// in the BFT system.
    #[cfg(feature = "serialize_serde")]
    type Reply: for<'a> Deserialize<'a> + Serialize + Send + Clone;

    #[cfg(feature = "serialize_capnp")]
    type Reply: Send + Clone;
    
    ///Serialize a request from your service, given the writer to serialize into
    ///  (either for network sending or persistent storing)
    fn serialize_request<W>(w: W, request: &Self::Request) -> Result<()> where W: Write;

    ///Deserialize a request that was generated by the serialize request function above
    ///  (either for network sending or persistent storing)
    fn deserialize_request<R>(r: R) -> Result<Self::Request> where R: Read;

    ///Serialize a reply into a given writer
    ///  (either for network sending or persistent storing)
    fn serialize_reply<W>(w: W, reply: &Self::Reply) -> Result<()> where W: Write;

    ///Deserialize a reply that was generated using the serialize reply function above
    ///  (either for network sending or persistent storing)
    fn deserialize_reply<R>(r: R) -> Result<Self::Reply> where R: Read;
}

/// Marker trait containing the types used by the application,
/// as well as routines to serialize the application data.
///
/// Both clients and replicas should implement this trait,
/// to communicate with each other.
/// This data type must be Send since it will be sent across
/// threads for processing and follow up reception
pub trait SharedData: Send {
    /// The application state, which is mutated by client
    /// requests.
    #[cfg(feature = "serialize_serde")]
    type State: for<'a> Deserialize<'a> + Serialize + Send + Clone;

    #[cfg(feature = "serialize_capnp")]
    type State: Send + Clone;

    /// Represents the requests forwarded to replicas by the
    /// clients of the BFT system.
    #[cfg(feature = "serialize_serde")]
    type Request: for<'a> Deserialize<'a> + Serialize + Send + Clone;

    #[cfg(feature = "serialize_capnp")]
    type Request: Send + Clone;

    /// Represents the replies forwarded to clients by replicas
    /// in the BFT system.
    #[cfg(feature = "serialize_serde")]
    type Reply: for<'a> Deserialize<'a> + Serialize + Send + Clone;

    #[cfg(feature = "serialize_capnp")]
    type Reply: Send + Clone;

    ///Serialize a state so it can be utilized by the SMR middleware
    ///  (either for network sending or persistent storing)
    fn serialize_state<W>(w: W, state: &Self::State) -> Result<()> where W: Write;

    ///Deserialize a state generated by the serialize_state function.
    ///  (either for network sending or persistent storing)
    fn deserialize_state<R>(r: R) -> Result<Self::State> where R: Read;

    ///Serialize a request from your service, given the writer to serialize into
    ///  (either for network sending or persistent storing)
    fn serialize_request<W>(w: W, request: &Self::Request) -> Result<()> where W: Write;

    ///Deserialize a request that was generated by the serialize request function above
    ///  (either for network sending or persistent storing)
    fn deserialize_request<R>(r: R) -> Result<Self::Request> where R: Read;

    ///Serialize a reply into a given writer
    ///  (either for network sending or persistent storing)
    fn serialize_reply<W>(w: W, reply: &Self::Reply) -> Result<()> where W: Write;

    ///Deserialize a reply that was generated using the serialize reply function above
    ///  (either for network sending or persistent storing)
    fn deserialize_reply<R>(r: R) -> Result<Self::Reply> where R: Read;
}

pub trait 

pub fn digest_state<D: SharedData>(appstate: &D::State) -> Result<Digest> {

    let mut state_vec = Vec::with_capacity(size_of::<D::State>());

    D::serialize_state(&mut state_vec, &appstate)?;

    let mut ctx = Context::new();

    ctx.update(&state_vec);

    let digest = ctx.finish();

    Ok(digest)
}
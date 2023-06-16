use serde::{Deserialize, Serialize};

/// The type abstraction for a monolithic state (only needs to be serializable, in reality)
#[cfg(feature = "serialize_serde")]
pub trait MonolithicState: for<'a> Deserialize<'a> + Serialize + Send + Clone {}

#[cfg(feature = "serialize_capnp")]
pub trait MonolithicState: Send + Clone {}
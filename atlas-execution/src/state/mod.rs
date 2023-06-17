#[cfg(feature = "serialize_serde")]
use serde::{Deserialize, Serialize};

pub mod divisible_state;
pub mod monolithic_state;

pub trait State {

    #[cfg(feature = "serialize_serde")]
    type State: for<'a> Deserialize<'a> + Serialize + Send + Clone;

    fn checkpoint(&self) -> Self::State;

}
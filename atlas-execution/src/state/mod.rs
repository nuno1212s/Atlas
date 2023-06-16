use serde::{Deserialize, Serialize};

pub mod divisible_state;
pub mod monolithic_state;

pub trait State {

    type State: for<'a> Deserialize<'a> + Serialize + Send + Clone;

    fn checkpoint(&self) -> Self::State;

}
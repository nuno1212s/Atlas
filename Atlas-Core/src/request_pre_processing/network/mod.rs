use atlas_common::error::*;
use atlas_communication::message::StoredMessage;
use std::time::Duration;

/// The abstraction for the network layer of the request pre-processing module.
pub trait RequestPreProcessingHandle<D>: Send + Sync {
    /// Receive requests from clients, block if there are no available requests
    fn receive_from_clients(&self, timeout: Option<Duration>) -> Result<Vec<StoredMessage<D>>>;

    /// Try to receive requests from clients, does not block if there are no available requests
    fn try_receive_from_clients(&self) -> Result<Option<Vec<StoredMessage<D>>>>;
}

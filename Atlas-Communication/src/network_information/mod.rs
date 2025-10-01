use tracing::{error, info};

use atlas_common::crypto::signature::PublicKey;
use atlas_common::error::*;
use atlas_common::node_id::{NodeId, NodeType};

use crate::reconfiguration::{
    NetworkReconfigurationCommunication, ReconfigurationNetworkUpdateMessage,
};

/// The trait required for a connection manager to be usable with the networking information
pub(crate) trait PendingConnectionManagement: Send {
    // Check if we have a pending connection to a given node
    fn has_pending_connection(&self, node: &NodeId) -> bool;

    // Upgrade a pending connection to a known connection, as we have received a confirmation from the reconfiguration protocol
    fn upgrade_connection_to_known(
        &self,
        node: &NodeId,
        node_type: NodeType,
        key: PublicKey,
    ) -> Result<()>;
}

/// Initialize the network information thread
pub(crate) fn initialize_network_info_handle<PM>(
    reconfiguration_message_handler: NetworkReconfigurationCommunication,
    connection_handler: PM,
) where
    PM: PendingConnectionManagement + 'static,
{
    std::thread::Builder::new()
        .name(String::from("Network Information Reception Thread"))
        .spawn(move || {
            loop {
                match reconfiguration_message_handler.receive_network_update() {
                    Ok(message) => {
                        match message {
                            ReconfigurationNetworkUpdateMessage::NodeConnectionPermitted(node, node_type, key) => {
                                if connection_handler.has_pending_connection(&node) {
                                    if let Err(err) = connection_handler.upgrade_connection_to_known(&node, node_type, key) {
                                        error!("Failed to upgrade connection to node {:?} of type {:?}. Error: {:?}", node, node_type, err);

                                        continue;
                                    }
                                    info!("Successfully upgraded connection to node {:?} of type {:?}", node, node_type);
                                } else {
                                    error!("Received a connection permitted message for a node that is not pending connection. Node: {:?}", node);
                                }
                            }
                        }
                    }
                    Err(err) => {
                        error!("Failed to receive network update message. Error: {:?}", err);

                        break;
                    }
                }
            }
        })
        .expect("Failed to launch network information reception thread");
}

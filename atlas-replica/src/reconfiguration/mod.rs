use std::sync::Arc;

use atlas_reconfiguration::QuorumNode;


pub struct ReconfigurableQuorumNode {

    node_data: Arc<QuorumNode>

}
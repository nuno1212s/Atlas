use crate::node_types::client::ClientQuorumView;

mod client;
mod replica;

pub enum NodeType {
    Client(ClientQuorumView),
    Replica()
}
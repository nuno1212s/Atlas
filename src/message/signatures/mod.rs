use atlas_common::crypto::signature::{KeyPair, PublicKey, Signature};
use atlas_common::error::*;
use atlas_communication::reconfiguration::NodeInfo;

fn encode_to_vec(node_triple: &NodeInfo, vec: &mut Vec<u8>) {
    bincode::serde::encode_into_std_write(node_triple, vec, bincode::config::standard()).unwrap();
}

pub(crate) fn create_node_triple_signature(
    node_triple: &NodeInfo,
    keypair: &KeyPair,
) -> Result<Signature> {
    let mut buf = Vec::new();

    encode_to_vec(node_triple, &mut buf);

    keypair.sign(&buf)
}

pub(crate) fn verify_node_triple_signature(
    node_triple: &NodeInfo,
    signature: &Signature,
    pub_key: &PublicKey,
) -> bool {
    let mut buf = Vec::new();

    encode_to_vec(node_triple, &mut buf);

    pub_key.verify(buf.as_slice(), signature).is_ok()
}

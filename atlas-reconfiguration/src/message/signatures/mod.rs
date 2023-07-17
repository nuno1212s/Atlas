use atlas_common::crypto::signature::{KeyPair, PublicKey, Signature};
use atlas_common::error::*;
use crate::message::NodeTriple;

fn encode_to_vec(node_triple: &NodeTriple, vec: &mut Vec<u8>) {
    bincode::serde::encode_into_std_write(node_triple,  vec, bincode::config::standard()).unwrap();
}

pub (crate) fn create_node_triple_signature(node_triple: &NodeTriple, keypair: &KeyPair) -> Result<Signature> {
    let mut buf = Vec::new();
    
    encode_to_vec(node_triple, &mut buf);

    keypair.sign(&buf)
}

pub (crate) fn verify_node_triple_signature(node_triple: &NodeTriple, signature: &Signature, pub_key: &PublicKey) -> bool {
    let mut buf = Vec::new();

    encode_to_vec(node_triple, &mut buf);
    
    pub_key.verify(buf.as_slice(), signature).is_ok()
}
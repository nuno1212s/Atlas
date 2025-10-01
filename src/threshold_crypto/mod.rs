use atlas_common::crypto::threshold_crypto::thold_crypto::dkg::DistributedKeyGenerator;
use atlas_common::crypto::threshold_crypto::{PrivateKeyPart, PublicKeyPart, PublicKeySet};
use atlas_common::node_id::NodeId;
use atlas_core::reconfiguration_protocol::QuorumThresholdCrypto;

mod network;
mod ordered_bcast;
mod threshold_negotiation;

/// The threshold crypto struct, which will be used to
/// handle all threshold crypto operations
pub struct ThresholdCrypto {
    // Our node ID
    our_id: NodeId,
    // Our index in the key exchange
    our_index: usize,

    // Get the current quorum
    // The vector to indicate the quorum that participated and
    // the order in which they participated, to obtain the indexes
    current_quorum: Vec<NodeId>,

    // The current public key set,
    // used to verify message signatures,
    // Along with combining them
    pk_set: PublicKeySet,

    // Our part of the private key,
    // Known only to us
    sk_part: PrivateKeyPart,
}

impl ThresholdCrypto {
    /// Initialize a threshold crypto instance from a distributed key generator
    pub fn from_finished_dkg(
        our_id: NodeId,
        quorum: Vec<NodeId>,
        dkg: DistributedKeyGenerator,
    ) -> Self {
        let our_index = dkg.our_id();

        let (pk_set, sk_part) = dkg.finalize().expect("Failed to finalize DKG");

        Self {
            our_id,
            our_index,
            current_quorum: quorum,
            pk_set: pk_set.into(),
            sk_part: sk_part.into(),
        }
    }

    fn our_id(&self) -> NodeId {
        self.our_id
    }

    fn our_index(&self) -> usize {
        self.our_index
    }

    fn id_of_node(&self, node: NodeId) -> usize {
        self.current_quorum.iter().position(|x| *x == node).unwrap() + 1
    }
}

impl QuorumThresholdCrypto for ThresholdCrypto {
    fn own_pub_key(&self) -> PublicKeyPart {
        self.pk_set.public_key_share(self.our_index)
    }

    fn pub_key_for_node(&self, node: NodeId) -> PublicKeyPart {
        self.pk_set.public_key_share(self.id_of_node(node))
    }

    fn pub_key_set(&self) -> atlas_common::error::Result<&PublicKeySet> {
        Ok(&self.pk_set)
    }

    fn get_priv_key_part(&self) -> atlas_common::error::Result<&PrivateKeyPart> {
        Ok(&self.sk_part)
    }
}

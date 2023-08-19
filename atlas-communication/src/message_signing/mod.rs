use std::marker::PhantomData;
use std::sync::Arc;
use intmap::IntMap;
use atlas_common::crypto::hash::{Context, Digest};
use atlas_common::crypto::signature::{KeyPair, PublicKey, Signature};
use atlas_common::error::*;
use crate::config::PKConfig;
use crate::message::{Header, WireMessage};
use crate::reconfiguration_node::NetworkInformationProvider;
use crate::serialize::{Buf, Serializable};

/// A trait that defines the signature verification function
pub trait NetworkMessageSignatureVerifier<M, NI>
    where M: Serializable, NI: NetworkInformationProvider, Self: Sized {

    /// Verify the signature of the internal message structure
    /// Returns Result<bool> where true means the signature is valid, false means it is not
    fn verify_signature(info_provider: &Arc<NI>, header: &Header, msg: &M::Message, buf: &Buf) -> Result<bool>;
}

pub struct DefaultSignatureVerifier<M: Serializable, NI: NetworkInformationProvider>(Arc<NI>, PhantomData<M>);

impl<M, NI> NetworkMessageSignatureVerifier<M, NI> for DefaultSignatureVerifier<M, NI>
    where M: Serializable, NI: NetworkInformationProvider {

    fn verify_signature(info_provider: &Arc<NI>, header: &Header, msg: &M::Message, buf: &Buf) -> Result<bool> {
        let key = info_provider.get_public_key(&header.from()).ok_or(Error::simple_with_msg(ErrorKind::CommunicationPeerNotFound, "Could not find public key for peer"))?;

        let sig = header.signature();

        if let Ok(()) = verify_parts(&key, sig, header.from().0 as u32, header.to().0 as u32, header.nonce(), &buf[..]) {
            M::verify_message_internal::<Self, NI>(info_provider, header, msg, buf)
        } else {
            Ok(false)
        }
    }
}

#[deprecated(since = "0.1.0", note = "please use `ReconfigurableNetworkNode` instead")]
pub struct NodePKShared {
    my_key: Arc<KeyPair>,
    peer_keys: IntMap<PublicKey>,
}

impl NodePKShared {
    pub fn from_config(config: PKConfig) -> Arc<Self> {
        Arc::new(Self {
            my_key: Arc::new(config.sk),
            peer_keys: config.pk,
        })
    }

    pub fn new(my_key: KeyPair, peer_keys: IntMap<PublicKey>) -> Self {
        Self {
            my_key: Arc::new(my_key),
            peer_keys,
        }
    }
}

#[derive(Clone)]
pub struct NodePKCrypto {
    pk_shared: Arc<NodePKShared>,
}

impl NodePKCrypto {
    pub fn new(pk_shared: Arc<NodePKShared>) -> Self {
        Self { pk_shared }
    }

    pub fn my_key(&self) -> &Arc<KeyPair> {
        &self.pk_shared.my_key
    }
}

fn digest_parts(from: u32, to: u32, nonce: u64, payload: &[u8]) -> Digest {
    let mut ctx = Context::new();

    let buf = WireMessage::CURRENT_VERSION.to_le_bytes();
    ctx.update(&buf[..]);

    let buf = from.to_le_bytes();
    ctx.update(&buf[..]);

    let buf = to.to_le_bytes();
    ctx.update(&buf[..]);

    let buf = nonce.to_le_bytes();
    ctx.update(&buf[..]);

    let buf = (payload.len() as u64).to_le_bytes();
    ctx.update(&buf[..]);

    ctx.update(payload);
    ctx.finish()
}

///Sign a given message, with the following passed parameters
/// From is the node that sent the message
/// to is the destination node
/// nonce is the none
/// and the payload is what we actually want to sign (in this case we will
/// sign the digest of the message, instead of the actual entire payload
/// since that would be quite slow)
pub(crate) fn sign_parts(
    sk: &KeyPair,
    from: u32,
    to: u32,
    nonce: u64,
    payload: &[u8],
) -> Signature {
    let digest = digest_parts(from, to, nonce, payload);
    // NOTE: unwrap() should always work, much like heap allocs
    // should always work
    sk.sign(digest.as_ref()).unwrap()
}

///Verify the signature of a given message, which contains
/// the following parameters
/// From is the node that sent the message
/// to is the destination node
/// nonce is the none
/// and the payload is what we actually want to sign (in this case we will
/// sign the digest of the message, instead of the actual entire payload
/// since that would be quite slow)
pub(crate) fn verify_parts(
    pk: &PublicKey,
    sig: &Signature,
    from: u32,
    to: u32,
    nonce: u64,
    payload: &[u8],
) -> Result<()> {
    let digest = digest_parts(from, to, nonce, payload);
    pk.verify(digest.as_ref(), sig)
}
use std::marker::PhantomData;
use std::sync::Arc;
use intmap::IntMap;
use atlas_common::crypto::hash::{Context, Digest};
use atlas_common::crypto::signature::{KeyPair, PublicKey, Signature};
use atlas_common::error::*;
use crate::config::PKConfig;
use crate::cpu_workers;
use crate::message::{Header, NetworkMessageKind, WireMessage};
use crate::reconfiguration_node::NetworkInformationProvider;
use crate::serialize::{Buf, digest_message, Serializable};

/// A trait that defines the signature verification function
pub trait NetworkMessageSignatureVerifier<M, NI>
    where M: Serializable, NI: NetworkInformationProvider, Self: Sized {
    fn verify_signature(info_provider: &Arc<NI>, header: &Header, msg: M::Message) -> Result<(bool, M::Message)>;

    /// Verify the signature of the internal message structure
    /// Returns Result<bool> where true means the signature is valid, false means it is not
    fn verify_signature_with_buf(info_provider: &Arc<NI>, header: &Header, msg: &M::Message, buf: &Buf) -> Result<bool>;
}

pub enum MessageKind {
    Reconfig,
    Protocol,
}

pub struct DefaultReconfigSignatureVerifier<RM: Serializable, PM: Serializable, NI: NetworkInformationProvider>(Arc<NI>, PhantomData<(RM, PM)>);

impl<PM, RM, NI> NetworkMessageSignatureVerifier<RM, NI> for DefaultReconfigSignatureVerifier<RM, PM, NI>
    where PM: Serializable, RM: Serializable, NI: NetworkInformationProvider + 'static {
    fn verify_signature(info_provider: &Arc<NI>, header: &Header, msg: RM::Message) -> Result<(bool, RM::Message)> {
        let key = info_provider.get_public_key(&header.from()).ok_or(Error::simple_with_msg(ErrorKind::CommunicationPeerNotFound, "Could not find public key for peer"))?;

        let sig = header.signature();

        let network = NetworkMessageKind::<RM, PM>::from_reconfig(msg);

        if let Ok((buf, digest)) = cpu_workers::serialize_digest_no_threadpool(&network) {
            if let Ok(()) = verify_parts(&key, sig, header.from().0 as u32, header.to().0 as u32, header.nonce(), digest.as_ref()) {
                if RM::verify_message_internal::<NI, Self>(info_provider, header, network.deref_reconfig())? {
                    Ok((true, network.into_reconfig()))
                } else {
                    Ok((false, network.into_reconfig()))
                }
            } else {
                Ok((false, network.into_reconfig()))
            }
        } else {
            Ok((false, network.into_reconfig()))
        }
    }

    fn verify_signature_with_buf(info_provider: &Arc<NI>, header: &Header, msg: &RM::Message, buf: &Buf) -> Result<bool> {
        let key = info_provider.get_public_key(&header.from()).ok_or(Error::simple_with_msg(ErrorKind::CommunicationPeerNotFound, "Could not find public key for peer"))?;

        let sig = header.signature();

        if let Ok(digest) = digest_message(buf.clone()) {
            if let Ok(()) = verify_parts(&key, sig, header.from().0 as u32, header.to().0 as u32, header.nonce(), digest.as_ref()) {
                RM::verify_message_internal::<NI, Self>(info_provider, header, msg)
            } else {
                Ok(false)
            }
        } else {
            Ok(false)
        }
    }
}

pub struct DefaultProtocolSignatureVerifier<RM: Serializable, PM: Serializable, NI: NetworkInformationProvider>(Arc<NI>, PhantomData<(RM, PM)>);

impl<RM, PM, NI> NetworkMessageSignatureVerifier<PM, NI> for DefaultProtocolSignatureVerifier<RM, PM, NI>
    where RM: Serializable, PM: Serializable, NI: NetworkInformationProvider + 'static {
    fn verify_signature(info_provider: &Arc<NI>, header: &Header, msg: PM::Message) -> Result<(bool, PM::Message)> {
        let key = info_provider.get_public_key(&header.from()).ok_or(Error::simple_with_msg(ErrorKind::CommunicationPeerNotFound, "Could not find public key for peer"))?;

        let sig = header.signature();

        let network = NetworkMessageKind::<RM, PM>::from_system(msg);

        if let Ok((buf, digest)) = cpu_workers::serialize_digest_no_threadpool(&network) {
            if let Ok(()) = verify_parts(&key, sig, header.from().0 as u32, header.to().0 as u32, header.nonce(), digest.as_ref()) {
                if PM::verify_message_internal::<NI, Self>(info_provider, header, network.deref_system())? {
                    Ok((true, network.into_system()))
                } else {
                    Ok((false, network.into_system()))
                }
            } else {
                Ok((false, network.into_system()))
            }
        } else {
            Ok((false, network.into_system()))
        }
    }

    fn verify_signature_with_buf(info_provider: &Arc<NI>, header: &Header, msg: &PM::Message, buf: &Buf) -> Result<bool> {
        let key = info_provider.get_public_key(&header.from()).ok_or(Error::simple_with_msg(ErrorKind::CommunicationPeerNotFound, "Could not find public key for peer"))?;

        let sig = header.signature();

        if let Ok(digest) = digest_message(buf.clone()) {
            if let Ok(()) = verify_parts(&key, sig, header.from().0 as u32, header.to().0 as u32, header.nonce(), digest.as_ref()) {
                PM::verify_message_internal::<NI, Self>(info_provider, header, msg)
            } else {
                Ok(false)
            }
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
    payload_digest: &[u8],
) -> Result<()> {
    let digest = digest_parts(from, to, nonce, payload_digest);
    pk.verify(digest.as_ref(), sig)
}
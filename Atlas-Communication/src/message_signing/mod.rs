use crate::message::{verify_validity, Buf, Header, MessageErrors, WireMessage};
use crate::reconfiguration::NetworkInformationProvider;
use crate::reconfiguration::NodeInfo;
use crate::serialization;
use crate::serialization::Serializable;
use atlas_common::crypto::hash::{Context, Digest};
use atlas_common::crypto::signature::{KeyPair, PublicKey, Signature, VerifyError};
use atlas_common::node_id::NodeId;
use atlas_common::Err;
use thiserror::Error;

/// Verify the validity of a message, for any serializable type.
/// Performs the serialization and follow up digest and signature verification of the byte message.
///
/// Guarantees AUTHENTICITY, INTEGRITY and NON-REPUDIATION of the message.
pub fn verify_message_validity<M>(
    network_info: &impl NetworkInformationProvider,
    header: &Header,
    message: &M::Message,
) -> atlas_common::error::Result<()>
where
    M: Serializable,
{
    let mut msg = Vec::new();

    serialization::serialize_message::<Vec<u8>, M>(&mut msg, message)?;

    let message_bytes = Buf::from(msg);

    verify_ser_message_validity(network_info, header, &message_bytes)?;

    Ok(())
}

/// Verifies the validity of a serialized network message
///
/// Guarantees AUTHENTICITY, INTEGRITY and NON-REPUDIATION of the message.
pub(crate) fn verify_ser_message_validity(
    network_info: &impl NetworkInformationProvider,
    header: &Header,
    message: &Buf,
) -> Result<(), IngestionError> {
    let digest = serialization::digest_message(message);

    if *header.digest() != digest {
        return Err!(IngestionError::DigestDoesNotMatch(digest, *header.digest()));
    }

    let node_info = network_info.get_node_info(&header.from());

    let pub_key = node_info.as_ref().map(NodeInfo::public_key);

    if let Some(key) = pub_key {
        verify_validity(header, message, true, Some(key))?;

        Ok(())
    } else {
        Err!(IngestionError::NodeNotKnown(header.from()))
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
) -> Result<(), VerifyError> {
    let digest = digest_parts(from, to, nonce, payload_digest);
    pk.verify(digest.as_ref(), sig)
}

/// The error type for ingestion of messages
#[derive(Error, Debug)]
pub enum IngestionError {
    #[error("The digest of the message {0:?} does not match the digest in the header {1:?}")]
    DigestDoesNotMatch(Digest, Digest),
    #[error("We do not know the node {0:?} (No Public key available)")]
    NodeNotKnown(NodeId),
    #[error("The message we have received has an invalid signature {0:?}")]
    InvalidSignature(#[from] MessageErrors),
}

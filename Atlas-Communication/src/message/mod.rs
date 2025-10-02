use crate::lookup_table::MessageModule;

use atlas_common::crypto::hash::Digest;
use atlas_common::crypto::signature::{KeyPair, PublicKey, Signature, VerifyError};
use atlas_common::node_id::NodeId;
use atlas_common::ordering::{Orderable, SeqNo};
use atlas_common::Err;
use bytes::Bytes;
use futures::{AsyncWrite, AsyncWriteExt};
use serde::{Deserialize, Serialize};
use std::fmt::{Debug, Formatter};
use std::io;
use std::io::Write;
use std::mem::MaybeUninit;
use thiserror::Error;

/// The buffer type used to serialize messages into.
pub type Buf = Bytes;

pub type NetworkSerializedMessage = WireMessage;

pub type StoredSerializedMessage<M> = StoredMessage<SerializedMessage<M>>;

pub struct SerializedMessage<M> {
    pub(crate) original: M,
    pub(crate) raw: Buf,
}

impl<M> SerializedMessage<M> {
    pub fn new(original: M, raw: Buf) -> Self {
        Self { original, raw }
    }

    pub fn original(&self) -> &M {
        &self.original
    }

    pub fn raw(&self) -> &Buf {
        &self.raw
    }

    pub fn into_inner(self) -> (M, Buf) {
        (self.original, self.raw)
    }
}

/// Contains a system message as well as its respective header.
/// Convenience type to allow to store messages more directly, instead of having
/// the entire network message wrapper (with type of message, etc)
#[cfg_attr(feature = "serialize_serde", derive(Serialize, Deserialize))]
pub struct StoredMessage<M> {
    pub(crate) header: Header,
    pub(crate) message: M,
}

impl<M> StoredMessage<M> {
    /// Constructs a new `StoredMessage`.
    pub fn new(header: Header, message: M) -> Self {
        Self { header, message }
    }

    /// Returns the stored message's header.
    pub fn header(&self) -> &Header {
        &self.header
    }

    /// Returns the stored system message.
    pub fn message(&self) -> &M {
        &self.message
    }

    /// Return the inner types of this `StoredMessage`.
    pub fn into_inner(self) -> (Header, M) {
        (self.header, self.message)
    }
}

impl<M> Orderable for StoredMessage<M>
where
    M: Orderable,
{
    fn sequence_number(&self) -> SeqNo {
        self.message().sequence_number()
    }
}

impl<M> Clone for StoredMessage<M>
where
    M: Clone,
{
    fn clone(&self) -> Self {
        Self {
            header: self.header,
            message: self.message.clone(),
        }
    }
}

impl<M> Debug for StoredMessage<M>
where
    M: Debug,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "StoredMessage {{ header: {:?}, message: {:?} }}",
            self.header, self.message
        )
    }
}

/// A header that is sent before a message in transit in the wire.
///
/// A fixed amount of `Header::LENGTH` bytes are read before
/// a message is read. Contains the protocol version, message
/// length, as well as other metadata.
#[derive(Copy, Clone, Eq, PartialEq)]
#[repr(C, packed)]
pub struct Header {
    // manually align memory for cross platform compat
    pub(crate) _align: u32,
    // the protocol version
    pub(crate) version: u32,
    // origin of the message
    pub(crate) from: u32,
    // destination of the message
    pub(crate) to: u32,
    // a random number
    pub(crate) nonce: u64,
    // length of the payload
    pub(crate) length: u64,
    // the digest of the serialized payload
    pub(crate) digest: [u8; Digest::LENGTH],
    // sign(hash(le(from) + le(to) + le(nonce) + hash(serialize(payload))))
    pub(crate) signature: [u8; Signature::LENGTH],
}

#[cfg(feature = "serialize_serde")]
impl serde::Serialize for Header {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        // TODO: improve this, to avoid allocating a `Vec`
        let mut bytes = vec![0; Self::LENGTH];
        let hdr: &[u8; Self::LENGTH] = unsafe { std::mem::transmute(self) };
        bytes.copy_from_slice(&hdr[..]);
        serde_bytes::serialize(&bytes, serializer)
    }
}

#[cfg(feature = "serialize_serde")]
impl<'de> serde::Deserialize<'de> for Header {
    fn deserialize<D>(deserializer: D) -> Result<Header, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let bytes: Vec<u8> = serde_bytes::deserialize(deserializer)?;
        let mut hdr: [u8; Self::LENGTH] = [0; Self::LENGTH];
        hdr.copy_from_slice(&bytes);
        Ok(unsafe { std::mem::transmute::<[u8; Self::LENGTH], Header>(hdr) })
    }
}

/// A message to be sent over the wire. The payload should be a serialized
/// `SystemMessage`, for correctness.
#[derive(Debug, Clone)]
pub struct WireMessage {
    pub(crate) header: Header,
    pub(crate) message_mod: MessageModule,
    pub(crate) payload: Bytes,
}

// FIXME: perhaps use references for serializing and deserializing,
// to save a stack allocation? probably overkill
impl Header {
    /// The size of the memory representation of the `Header` in bytes.
    pub const LENGTH: usize = size_of::<Self>();

    unsafe fn serialize_into_unchecked(self, buf: &mut [u8]) {
        #[cfg(target_endian = "big")]
        {
            self.version = self.version.to_le();
            self.nonce = self.nonce.to_le();
            self.from = self.from.to_le();
            self.to = self.to.to_le();
            self.length = self.length.to_le();
        }
        let hdr: [u8; Self::LENGTH] = std::mem::transmute::<Header, [u8; Self::LENGTH]>(self);
        buf[..Self::LENGTH].copy_from_slice(&hdr[..]);
    }

    /// Serialize a `Header` into a byte buffer of appropriate size.
    pub fn serialize_into(self, buf: &mut [u8]) -> Result<(), MessageErrors> {
        if buf.len() < Self::LENGTH {
            return Err!(MessageErrors::InvalidSizeSerDest(buf.len()));
        }
        unsafe { self.serialize_into_unchecked(buf) };
        Ok(())
    }

    unsafe fn deserialize_from_unchecked(buf: &[u8]) -> Self {
        let mut hdr: [u8; Self::LENGTH] = {
            let hdr = MaybeUninit::uninit();
            hdr.assume_init()
        };
        hdr[..].copy_from_slice(&buf[..Self::LENGTH]);
        #[cfg(target_endian = "big")]
        {
            hdr.version = hdr.version.to_be();
            hdr.nonce = hdr.nonce.to_be();
            hdr.from = hdr.from.to_be();
            hdr.to = hdr.to.to_le();
            hdr.length = hdr.length.to_be();
        }
        std::mem::transmute(hdr)
    }

    /// Deserialize a `Header` from a byte buffer of appropriate size.
    pub fn deserialize_from(buf: &[u8]) -> Result<Self, MessageErrors> {
        if buf.len() < Self::LENGTH {
            return Err!(MessageErrors::InvalidSizeHeader(buf.len()));
        }
        Ok(unsafe { Self::deserialize_from_unchecked(buf) })
    }

    /// Reports the current version of the wire protocol,
    /// i.e. `WireMessage::CURRENT_VERSION`.
    pub fn version(&self) -> u32 {
        self.version
    }

    /// The originating `NodeId`.
    pub fn from(&self) -> NodeId {
        self.from.into()
    }

    /// The destination `NodeId`.
    pub fn to(&self) -> NodeId {
        self.to.into()
    }

    /// The length of the payload associated with this `Header`.
    pub fn payload_length(&self) -> usize {
        self.length as usize
    }

    /// The signature of this `Header` and associated payload.
    pub fn signature(&self) -> &Signature {
        unsafe { std::mem::transmute(&self.signature) }
    }

    /// The digest of the associated payload serialized data.
    pub fn digest(&self) -> &Digest {
        unsafe { std::mem::transmute(&self.digest) }
    }

    /// Hashes the digest of the associated message's payload
    /// with this header's nonce.
    ///
    /// This is useful for attaining a unique identifier for
    /// a particular client request.
    pub fn unique_digest(&self) -> Digest {
        self.digest().entropy(self.nonce.to_le_bytes())
    }

    /// Returns the nonce associated with this `Header`.
    pub fn nonce(&self) -> u64 {
        self.nonce
    }
}

impl WireMessage {
    /// The current version of the wire protocol.
    pub const CURRENT_VERSION: u32 = 0;

    /// Wraps a `Header` and a byte array payload into a `WireMessage`.
    pub fn from_parts(
        header: Header,
        message_mod: MessageModule,
        payload: Buf,
    ) -> std::result::Result<Self, MessageErrors> {
        let wm = Self {
            header,
            message_mod,
            payload,
        };

        wm.is_valid(None, true)?;

        Ok(wm)
    }

    pub fn from_header(
        header: Header,
        message_mod: MessageModule,
    ) -> std::result::Result<Self, MessageErrors> {
        let wm = Self {
            header,
            message_mod,
            payload: Buf::new(),
        };

        wm.is_valid(None, false)?;

        Ok(wm)
    }

    /// Constructs a new message to be sent over the wire.
    pub fn new(
        from: NodeId,
        to: NodeId,
        message_mod: MessageModule,
        payload: Buf,
        nonce: u64,
        digest: Option<Digest>,
        sk: Option<&KeyPair>,
    ) -> Self {
        let digest = digest
            // safety: digests have repr(transparent)
            .map(|d| unsafe { std::mem::transmute(d) })
            // if payload length is 0
            .unwrap_or([0; Digest::LENGTH]);

        let signature = sk
            .map(|sk| {
                let signature = crate::message_signing::sign_parts(
                    sk,
                    from.into(),
                    to.into(),
                    nonce,
                    &digest[..],
                );
                // safety: signatures have repr(transparent)
                unsafe { std::mem::transmute(signature) }
            })
            .unwrap_or([0; Signature::LENGTH]);

        let (from, to) = (from.into(), to.into());

        let header = Header {
            _align: 0,
            version: Self::CURRENT_VERSION,
            length: payload.len() as u64,
            signature,
            digest,
            nonce,
            from,
            to,
        };

        Self {
            header,
            message_mod,
            payload,
        }
    }

    /// Retrieve the inner `Header` and payload byte buffer stored
    /// inside the `WireMessage`.
    pub fn into_inner(self) -> (Header, MessageModule, Buf) {
        (self.header, self.message_mod, self.payload)
    }

    pub fn message_module(&self) -> &MessageModule {
        &self.message_mod
    }

    /// Returns a reference to the `Header` of the `WireMessage`.
    pub fn header(&self) -> &Header {
        &self.header
    }

    /// Returns a reference to the payload bytes of the `WireMessage`.
    pub fn payload(&self) -> &[u8] {
        &self.payload
    }

    /// Returns a reference to the payload bytes of the `WireMessage`.
    pub fn payload_buf(&self) -> &Buf {
        &self.payload
    }

    /// Checks for the correctness of the `WireMessage`. This implies
    /// checking its signature, if a `PublicKey` is provided.
    pub fn is_valid(
        &self,
        public_key: Option<&PublicKey>,
        check_payload_len: bool,
    ) -> std::result::Result<(), MessageErrors> {
        verify_validity(&self.header, &self.payload, check_payload_len, public_key)
    }

    /// Serialize a `WireMessage` into an async writer.
    pub async fn write_to<W: AsyncWrite + Unpin>(&self, mut w: W, flush: bool) -> io::Result<()> {
        let mut buf = [0; Header::LENGTH];
        self.header.serialize_into(&mut buf[..]).unwrap();

        w.write_all(&buf[..]).await?;

        if !self.payload.is_empty() {
            w.write_all(&self.payload[..]).await?;
        }

        if flush {
            w.flush().await?;
        }

        Ok(())
    }

    /// Serialize a `WireMessage` into a sync writer.
    pub fn write_to_sync<W: Write>(&self, mut w: W, flush: bool) -> io::Result<()> {
        let mut buf = [0; Header::LENGTH];
        self.header.serialize_into(&mut buf[..]).unwrap();

        w.write_all(&buf[..])?;

        if !self.payload.is_empty() {
            w.write_all(&self.payload[..])?;
        }

        if flush {
            w.flush()?;
        }

        Ok(())
    }
}

pub fn verify_validity(
    header: &Header,
    message: &Buf,
    check_payload_len: bool,
    public_key: Option<&PublicKey>,
) -> std::result::Result<(), MessageErrors> {
    let preliminary_check_failed = header.version != WireMessage::CURRENT_VERSION
        || (check_payload_len && header.length != message.len() as u64);

    if preliminary_check_failed {
        return Err!(MessageErrors::InvalidWireMessage);
    }

    public_key
        .map(|pk| {
            crate::message_signing::verify_parts(
                pk,
                header.signature(),
                header.from,
                header.to,
                header.nonce,
                &header.digest[..],
            )
        })
        .unwrap_or(Ok(()))
        .map_err(|e| e.into())
}

impl Debug for Header {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let version = self.version;
        let length = self.length;
        let signature = self.signature.chunks(4).next().unwrap();
        let digest = self.digest.chunks(4).next().unwrap();
        let nonce = self.nonce;
        let from = self.from;
        let to = self.to;

        write!(f, "Header {{ version: {version}, length: {length}, signature: {signature:x?}, digest: {digest:x?}, nonce: {nonce}, from: {from}, to: {to} }}"
        )
    }
}

#[derive(Debug, Error)]
pub enum MessageErrors {
    #[error("The wire message is not valid")]
    InvalidWireMessage,
    #[error("Verification error {0:?}")]
    VerificationError(#[from] VerifyError),
    #[error("The header has an invalid size {0}")]
    InvalidSizeHeader(usize),
    #[error("Destination header is too small {0}")]
    InvalidSizeSerDest(usize),
}

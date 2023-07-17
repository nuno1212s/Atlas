//! Public key cryptographic operations.

use std::fmt::{Debug, Formatter};
#[cfg(feature = "serialize_serde")]
use serde::{Serialize, Deserialize};

use crate::error::*;

#[cfg(feature = "crypto_signature_ring_ed25519")]
mod ring_ed25519;

/// A `KeyPair` holds both the private and public key components
/// that form a digital identity.
pub struct KeyPair {
    #[cfg(feature = "crypto_signature_ring_ed25519")]
    inner: ring_ed25519::KeyPair,

    pub_key_bytes: Vec<u8>,
}

/// The public component of a `KeyPair`.
#[derive(Clone)]
pub struct PublicKey {
    #[cfg(feature = "crypto_signature_ring_ed25519")]
    inner: ring_ed25519::PublicKey,

    pk_bytes: Vec<u8>,
}

/// Reference to a `PublicKey`.
pub struct PublicKeyRef<'a> {
    #[cfg(feature = "crypto_signature_ring_ed25519")]
    inner: &'a ring_ed25519::PublicKey,

    byte_repr: &'a Vec<u8>,
}

/// A `Signature` is the result of using `KeyPair::sign`. Represents
/// a digital signature with a private key.
//
// FIXME: is it secure to derive PartialEq+Eq? maybe roll our own impl,
// using something like this:
//
// https://golang.org/src/crypto/subtle/constant_time.go?s=505:546#L2
//
#[derive(Copy, Clone, Eq, PartialEq, Hash)]
#[repr(transparent)]
#[cfg_attr(feature = "serialize_serde", derive(Serialize, Deserialize))]
pub struct Signature {
    #[cfg(feature = "crypto_signature_ring_ed25519")]
    inner: ring_ed25519::Signature,
}

impl KeyPair {
    /// Constructs a `KeyPair` from a byte buffer of appropriate size.
    pub fn from_bytes(raw_bytes: &[u8]) -> Result<Self> {
        let (inner, pk_bytes) = {
            #[cfg(feature = "crypto_signature_ring_ed25519")]
            { ring_ed25519::KeyPair::from_bytes(raw_bytes)? }
        };
        Ok(KeyPair { inner, pub_key_bytes: pk_bytes })
    }

    /// Returns a reference to the public component of this `KeyPair`.
    ///
    /// The returned key can be cloned into an owned type with `into()`,
    /// yielding a `PublicKey`.
    pub fn public_key<'a>(&'a self) -> PublicKeyRef<'a> {
        let inner = self.inner.public_key();
        PublicKeyRef { inner, byte_repr: &self.pub_key_bytes }
    }

    /// Returns a reference to the public key bytes of this `KeyPair`.
    /// This is used mostly for serialization stuff
    pub fn public_key_bytes(&self) -> &[u8] {
        &self.pub_key_bytes
    }

    /// Performs a cryptographic signature of an arbitrary message.
    ///
    /// The hash of the message is calculated by `sign()`, so the users
    /// don't need to perform this step themselves.
    pub fn sign(&self, message: &[u8]) -> Result<Signature> {
        let inner = self.inner.sign(message)?;
        Ok(Signature { inner })
    }
}

impl<'a> From<PublicKeyRef<'a>> for PublicKey {
    fn from(pk: PublicKeyRef<'a>) -> PublicKey {
        let inner = pk.inner.clone();
        PublicKey { inner, pk_bytes: pk.byte_repr.clone() }
    }
}

impl<'a> PublicKeyRef<'a> {
    /// Check the `verify` documentation for `PublicKey`.
    pub fn verify(&self, message: &[u8], signature: &Signature) -> Result<()> {
        self.inner.verify(message, &signature.inner)
    }
}

impl PublicKey {
    /// Constructs a `PublicKey` from a byte buffer of appropriate size.
    pub fn from_bytes(raw_bytes: &[u8]) -> Result<Self> {
        let inner = {
            #[cfg(feature = "crypto_signature_ring_ed25519")]
            { ring_ed25519::PublicKey::from_bytes(raw_bytes)? }
        };
        Ok(PublicKey { inner, pk_bytes: raw_bytes.to_vec() })
    }

    pub fn pk_bytes(&self) -> &[u8] {
        &self.pk_bytes
    }

    /// Verifies if a signature is valid, i.e. if this `KeyPair` performed it.
    ///
    /// Forged signatures can be verified successfully, so a good public key
    /// crypto algorithm and key size should be picked.
    pub fn verify(&self, message: &[u8], signature: &Signature) -> Result<()> {
        self.inner.verify(message, &signature.inner)
    }
}

impl Signature {
    /// Length in bytes required to represent a `Signature` in memory.
    pub const LENGTH: usize = {
        #[cfg(feature = "crypto_signature_ring_ed25519")]
        { ring_ed25519::Signature::LENGTH }
    };

    /// Constructs a `Signature` from a byte buffer of appropriate size.
    pub fn from_bytes(raw_bytes: &[u8]) -> Result<Self> {
        let inner = {
            #[cfg(feature = "crypto_signature_ring_ed25519")]
            { ring_ed25519::Signature::from_bytes(raw_bytes)? }
        };
        Ok(Signature { inner })
    }
}

impl Debug for Signature {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:x?}", self.inner.as_ref().chunks(4).next().unwrap())
    }
}

impl AsRef<[u8]> for Signature {
    fn as_ref(&self) -> &[u8] {
        self.inner.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::Signature;

    #[test]
    fn test_length() {
        assert_eq!(Signature::LENGTH, std::mem::size_of::<Signature>());
    }
}
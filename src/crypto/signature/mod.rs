//! Public key cryptographic operations.

#[cfg(feature = "serialize_serde")]
use serde::de::{Error, SeqAccess};
#[cfg(feature = "serialize_serde")]
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::cmp;
use std::fmt::{Debug, Formatter};
use thiserror::Error;

use crate::error::*;

#[cfg(feature = "crypto_signature_ring_ed25519")]
mod ring_ed25519;

#[derive(Error, Debug)]
pub enum SignError {
    #[error("Failed to generate key {0:?}")]
    GenerateKey(String),
    #[error("Invalid signature {0:?}")]
    InvalidSignature(String),
    #[error("Invalid private key {0:?}")]
    InvalidPK(String),
    #[error("Invalid public key, length is wrong {0}")]
    PublicKeyLen(usize),
    #[error("Invalid signature, length is wrong {0}")]
    SignatureLen(usize),
}

#[derive(Error, Debug)]
pub enum VerifyError {
    #[error("Failed too verify signature {0:?}, signature is {1:x?}")]
    VerificationError(String, Vec<u8>),
    #[error("Invalid signature, cannot be blank")]
    BlankSignature,
    #[error("Invalid signature, length is wrong {0}")]
    SignatureLen(usize),
}

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
    pub fn generate_key_pair() -> Result<Self> {
        let (inner, public_key) = {
            #[cfg(feature = "crypto_signature_ring_ed25519")]
            {
                ring_ed25519::KeyPair::generate()?
            }
        };

        Ok(KeyPair {
            inner,
            pub_key_bytes: public_key,
        })
    }

    pub fn from_pkcs8(bytes: &[u8]) -> Result<Self> {
        let (inner, public_key) = {
            #[cfg(feature = "crypto_signature_ring_ed25519")]
            {
                ring_ed25519::KeyPair::from_pkcs8(bytes)?
            }
        };

        Ok(KeyPair {
            inner,
            pub_key_bytes: public_key,
        })
    }

    /// Constructs a `KeyPair` from a byte buffer of appropriate size.
    pub fn from_bytes(raw_bytes: &[u8]) -> Result<Self> {
        let (inner, pk_bytes) = {
            #[cfg(feature = "crypto_signature_ring_ed25519")]
            {
                ring_ed25519::KeyPair::from_bytes(raw_bytes)?
            }
        };
        Ok(KeyPair {
            inner,
            pub_key_bytes: pk_bytes,
        })
    }

    /// Returns a reference to the public component of this `KeyPair`.
    ///
    /// The returned key can be cloned into an owned type with `into()`,
    /// yielding a `PublicKey`.
    pub fn public_key(&self) -> PublicKeyRef<'_> {
        let inner = self.inner.public_key();
        PublicKeyRef {
            inner,
            byte_repr: &self.pub_key_bytes,
        }
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
        let inner = *pk.inner;
        PublicKey {
            inner,
            pk_bytes: pk.byte_repr.clone(),
        }
    }
}

impl<'a> PublicKeyRef<'a> {
    /// Check the `verify` documentation for `PublicKey`.
    pub fn verify(
        &self,
        message: &[u8],
        signature: &Signature,
    ) -> std::result::Result<(), VerifyError> {
        self.inner.verify(message, &signature.inner)
    }
}

impl PublicKey {
    /// Constructs a `PublicKey` from a byte buffer of appropriate size.
    pub fn from_bytes(raw_bytes: &[u8]) -> Result<Self> {
        let inner = {
            #[cfg(feature = "crypto_signature_ring_ed25519")]
            {
                ring_ed25519::PublicKey::from_bytes(raw_bytes)?
            }
        };

        Ok(PublicKey {
            inner,
            pk_bytes: raw_bytes.to_vec(),
        })
    }

    pub fn pk_bytes(&self) -> &[u8] {
        &self.pk_bytes
    }

    /// Verifies if a signature is valid, i.e. if this `KeyPair` performed it.
    ///
    /// Forged signatures can be verified successfully, so a good public key
    /// crypto algorithm and key size should be picked.
    pub fn verify(
        &self,
        message: &[u8],
        signature: &Signature,
    ) -> std::result::Result<(), VerifyError> {
        self.inner.verify(message, &signature.inner)
    }
}

impl Signature {
    /// Length in bytes required to represent a `Signature` in memory.
    pub const LENGTH: usize = {
        #[cfg(feature = "crypto_signature_ring_ed25519")]
        {
            ring_ed25519::Signature::LENGTH
        }
    };

    /// Constructs a `Signature` from a byte buffer of appropriate size.
    pub fn from_bytes(raw_bytes: &[u8]) -> Result<Self> {
        let inner = {
            #[cfg(feature = "crypto_signature_ring_ed25519")]
            {
                ring_ed25519::Signature::from_bytes(raw_bytes)?
            }
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

impl Serialize for PublicKey {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_bytes(&self.pk_bytes)
    }
}

impl<'de> Deserialize<'de> for PublicKey {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct ByteBufVisitor;

        impl<'de> serde::de::Visitor<'de> for ByteBufVisitor {
            type Value = Vec<u8>;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a byte array")
            }

            fn visit_seq<V>(self, mut visitor: V) -> std::result::Result<Vec<u8>, V::Error>
            where
                V: SeqAccess<'de>,
            {
                let len = cmp::min(visitor.size_hint().unwrap_or(0), 4096);
                let mut bytes = Vec::with_capacity(len);

                while let Some(b) = visitor.next_element()? {
                    bytes.push(b);
                }

                Ok(bytes)
            }

            fn visit_bytes<E>(self, v: &[u8]) -> std::result::Result<Vec<u8>, E>
            where
                E: Error,
            {
                Ok(v.to_vec())
            }

            fn visit_byte_buf<E>(self, v: Vec<u8>) -> std::result::Result<Vec<u8>, E>
            where
                E: Error,
            {
                Ok(v)
            }
        }

        let vec = deserializer.deserialize_bytes(ByteBufVisitor)?;

        match Self::from_bytes(vec.as_slice()) {
            Ok(pk) => Ok(pk),
            Err(err) => Err(serde::de::Error::custom(err)),
        }
    }
}

impl Debug for PublicKey {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:x?}", self.pk_bytes.chunks(4).next().unwrap())
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

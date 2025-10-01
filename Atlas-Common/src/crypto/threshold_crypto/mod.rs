pub mod thold_crypto;
//mod frost;

use crate::crypto::threshold_crypto::thold_crypto::SecretKeySet;
#[cfg(feature = "serialize_serde")]
use serde::{Deserialize, Serialize};
use thiserror::Error;
use threshold_crypto::{Fr, IntoFr};

#[derive(Debug, Clone, Eq, PartialEq)]
#[repr(transparent)]
#[cfg_attr(feature = "serialize_serde", derive(Serialize, Deserialize))]
pub struct PublicKeyPart {
    key: thold_crypto::PublicKeyPart,
}

#[derive(Debug, Clone, Eq, PartialEq)]
#[repr(transparent)]
pub struct PrivateKeyPart {
    key: thold_crypto::PrivateKeyPart,
}

#[derive(Debug, Clone, Eq, PartialEq)]
#[repr(transparent)]
#[cfg_attr(feature = "serialize_serde", derive(Serialize, Deserialize))]
pub struct SerializableKeyPart {
    key: thold_crypto::SerializableKeyPart,
}

#[derive(Debug, Clone, Eq, PartialEq)]
#[repr(transparent)]
#[cfg_attr(feature = "serialize_serde", derive(Serialize, Deserialize))]
pub struct PublicKeySet {
    key: thold_crypto::PublicKeySet,
}

#[derive(Debug, Clone, Eq, PartialEq)]
#[repr(transparent)]
#[cfg_attr(feature = "serialize_serde", derive(Serialize, Deserialize))]
pub struct PublicKey {
    key: thold_crypto::PublicKey,
}

#[derive(Clone, Eq, PartialEq)]
#[repr(transparent)]
pub struct PrivateKeySet {
    key: thold_crypto::SecretKeySet,
}

#[derive(Clone, Eq, PartialEq, Hash, Debug)]
#[repr(transparent)]
#[cfg_attr(feature = "serialize_serde", derive(Serialize, Deserialize))]
pub struct PartialSignature {
    sig: thold_crypto::PartialSignature,
}

#[derive(Clone, Eq, PartialEq, Hash, Debug)]
#[repr(transparent)]
#[cfg_attr(feature = "serialize_serde", derive(Serialize, Deserialize))]
pub struct CombinedSignature {
    sig: thold_crypto::Signature,
}

impl PublicKey {}

#[allow(dead_code)]
impl PublicKeySet {
    pub fn public_key(&self) -> PublicKey {
        PublicKey {
            key: self.key.public_key(),
        }
    }

    pub fn public_key_share(&self, index: usize) -> PublicKeyPart {
        PublicKeyPart {
            key: self.key.get_public_key_part(index),
        }
    }

    pub fn verify(&self, msg: &[u8], sig: &CombinedSignature) -> Result<(), VerifySignatureError> {
        self.key.verify_combined_signature(msg, &sig.sig)?;

        Ok(())
    }

    pub fn verify_partial_signature(
        &self,
        id: usize,
        msg: &[u8],
        sig: &PartialSignature,
    ) -> Result<(), VerifySignatureError> {
        self.key.verify_partial_signature(id, msg, &sig.sig)
    }

    pub fn combine_signatures<'a, T, I>(
        &self,
        sigs: I,
    ) -> Result<CombinedSignature, CombineSignatureError>
    where
        I: IntoIterator<Item = (T, &'a PartialSignature)>,
        T: IntoFr,
    {
        let map = sigs
            .into_iter()
            .map(|(id, sig)| (id, &sig.sig))
            .collect::<Vec<_>>();

        Ok(CombinedSignature {
            sig: self.key.combine_signatures(map)?,
        })
    }
}

impl PublicKeyPart {
    pub fn verify(&self, msg: &[u8], sig: &PartialSignature) -> Result<(), VerifySignatureError> {
        self.key.verify(msg, &sig.sig)
    }
}

impl PrivateKeyPart {
    pub fn public_key_part(&self) -> PublicKeyPart {
        PublicKeyPart {
            key: self.key.public_key_part(),
        }
    }

    pub fn partially_sign(&self, msg: &[u8]) -> PartialSignature {
        PartialSignature {
            sig: self.key.partially_sign(msg),
        }
    }

    pub fn from_mut(sk: &mut Fr) -> Self {
        PrivateKeyPart {
            key: thold_crypto::PrivateKeyPart::from_mut(sk),
        }
    }
}

impl PrivateKeySet {
    /// Generate a new random private key set.
    /// Receives the threshold for the private key set.
    /// To combine signatures, we need at least `threshold` + 1 signatures.
    pub fn gen_random(threshold: usize) -> Self {
        let key = SecretKeySet::generate_random(threshold);

        Self { key }
    }

    pub fn public_key_set(&self) -> PublicKeySet {
        PublicKeySet {
            key: self.key.public_key_set(),
        }
    }

    pub fn private_key_part(&self, index: usize) -> PrivateKeyPart {
        let key_share = self.key.get_key_share(index);

        PrivateKeyPart { key: key_share }
    }
}

#[derive(Error, Debug)]
pub enum VerifySignatureError {
    #[error("The signature is not valid for this message")]
    WrongSignature,
}

#[derive(Error, Debug)]
pub enum CombineSignatureError {
    /// Not enough signature shares.
    #[error("Not enough signature shares")]
    NotEnoughShares,
    /// Signature shares contain a duplicated index.
    #[error("Signature shares contain a duplicated index")]
    DuplicateEntry,
    /// The degree is too high for the coefficients to be indexed by `usize`.
    #[error("The degree is too high for the coefficients to be indexed by usize.")]
    DegreeTooHigh,
}

#[derive(Error, Debug)]
pub enum ParsePublicKeyError {
    #[error("The public key is not valid")]
    InvalidPublicKey,
}

impl From<PrivateKeyPart> for SerializableKeyPart {
    fn from(pk: PrivateKeyPart) -> SerializableKeyPart {
        SerializableKeyPart { key: pk.key.into() }
    }
}

impl From<SerializableKeyPart> for PrivateKeyPart {
    fn from(pk: SerializableKeyPart) -> PrivateKeyPart {
        PrivateKeyPart { key: pk.key.into() }
    }
}

use crate::crypto::threshold_crypto::{
    CombineSignatureError, ParsePublicKeyError, VerifySignatureError,
};
#[cfg(feature = "serialize_serde")]
use serde::{Deserialize, Serialize};
use threshold_crypto::error::{Error, FromBytesError};
use threshold_crypto::poly::Commitment;
use threshold_crypto::serde_impl::SerdeSecret;
use threshold_crypto::{Fr, IntoFr, SecretKeyShare};

pub mod dkg;
//mod async_dkg;

#[derive(Debug, Clone, Eq, PartialEq)]
#[repr(transparent)]
#[cfg_attr(feature = "serialize_serde", derive(Serialize, Deserialize))]
pub(super) struct PublicKey {
    key: threshold_crypto::PublicKey,
}

#[derive(Debug, Clone, Eq, PartialEq)]
#[repr(transparent)]
#[cfg_attr(feature = "serialize_serde", derive(Serialize, Deserialize))]
pub(super) struct PublicKeyPart {
    key: threshold_crypto::PublicKeyShare,
}

#[derive(Debug, Clone, Eq, PartialEq)]
#[repr(transparent)]
pub(super) struct PrivateKeyPart {
    key: SecretKeyShare,
}

#[derive(Debug, Clone, Eq, PartialEq)]
#[repr(transparent)]
#[cfg_attr(feature = "serialize_serde", derive(Serialize, Deserialize))]
pub(super) struct SerializableKeyPart {
    key: SerdeSecret<SecretKeyShare>,
}

#[derive(Clone, Eq, PartialEq, Hash, Debug)]
#[repr(transparent)]
#[cfg_attr(feature = "serialize_serde", derive(Serialize, Deserialize))]
pub(super) struct PartialSignature {
    sig: threshold_crypto::SignatureShare,
}

#[derive(Clone, Eq, PartialEq, Hash, Debug)]
#[repr(transparent)]
#[cfg_attr(feature = "serialize_serde", derive(Serialize, Deserialize))]
pub(super) struct Signature {
    sig: threshold_crypto::Signature,
}

#[derive(Clone, Eq, PartialEq)]
#[repr(transparent)]
pub(super) struct SecretKeySet {
    sk_set: threshold_crypto::SecretKeySet,
}

/// The public key set
#[derive(Debug, Clone, Eq, PartialEq)]
#[repr(transparent)]
#[cfg_attr(feature = "serialize_serde", derive(Serialize, Deserialize))]
pub(super) struct PublicKeySet {
    pk_set: threshold_crypto::PublicKeySet,
}

impl PublicKeyPart {
    pub fn verify(&self, msg: &[u8], sig: &PartialSignature) -> Result<(), VerifySignatureError> {
        if self.key.verify(&sig.sig, msg) {
            Ok(())
        } else {
            Err(VerifySignatureError::WrongSignature)
        }
    }
}

impl SecretKeySet {
    pub fn generate_random(n: usize) -> SecretKeySet {
        let mut rand = rand::rngs::OsRng;

        let sk_set = threshold_crypto::SecretKeySet::random(n, &mut rand);

        SecretKeySet { sk_set }
    }

    pub(super) fn get_key_share(&self, i: usize) -> PrivateKeyPart {
        let key = self.sk_set.secret_key_share(i);

        PrivateKeyPart { key }
    }

    pub fn public_key_set(&self) -> PublicKeySet {
        PublicKeySet {
            pk_set: self.sk_set.public_keys(),
        }
    }
}

impl PrivateKeyPart {
    pub fn public_key_part(&self) -> PublicKeyPart {
        PublicKeyPart {
            key: self.key.public_key_share(),
        }
    }

    pub fn partially_sign(&self, msg: &[u8]) -> PartialSignature {
        let sig = self.key.sign(msg);

        PartialSignature { sig }
    }

    pub fn from_mut(sk: &mut Fr) -> Self {
        let key = threshold_crypto::SecretKeyShare::from_mut(sk);

        PrivateKeyPart { key }
    }
}

#[allow(dead_code)]
impl PublicKey {
    #[inline]
    pub fn verify_combined_signatures(&self, sig: &Signature, msg: &[u8]) -> bool {
        self.key.verify(&sig.sig, msg)
    }

    #[inline]
    pub fn to_bytes(&self) -> [u8; threshold_crypto::PK_SIZE] {
        self.key.to_bytes()
    }

    #[inline]
    pub fn from_bytes(
        bytes: &[u8; threshold_crypto::PK_SIZE],
    ) -> Result<PublicKey, ParsePublicKeyError> {
        threshold_crypto::PublicKey::from_bytes(bytes)
            .map(|key| PublicKey { key })
            .map_err(|err| match err {
                FromBytesError::Invalid => ParsePublicKeyError::InvalidPublicKey,
            })
    }
}

impl PublicKeySet {
    #[inline(always)]
    pub(super) fn public_key(&self) -> PublicKey {
        let key = self.pk_set.public_key();

        PublicKey { key }
    }

    #[inline(always)]
    pub(super) fn get_public_key_part(&self, i: usize) -> PublicKeyPart {
        let key = self.pk_set.public_key_share(i);

        PublicKeyPart { key }
    }

    #[inline(always)]
    pub(super) fn verify_partial_signature(
        &self,
        index: usize,
        msg: &[u8],
        sig: &PartialSignature,
    ) -> Result<(), VerifySignatureError> {
        self.get_public_key_part(index).verify(msg, sig)
    }

    #[inline(always)]
    pub(super) fn combine_signatures<'a, T, I>(
        &self,
        sigs: I,
    ) -> Result<Signature, CombineSignatureError>
    where
        I: IntoIterator<Item = (T, &'a PartialSignature)>,
        T: IntoFr,
    {
        let sigs = sigs
            .into_iter()
            .map(|(index, sign)| (index, &sign.sig))
            .collect::<Vec<_>>();

        let sig = self
            .pk_set
            .combine_signatures(sigs)
            .map_err(|err| match err {
                Error::NotEnoughShares => CombineSignatureError::NotEnoughShares,
                Error::DuplicateEntry => CombineSignatureError::DuplicateEntry,
                Error::DegreeTooHigh => CombineSignatureError::DegreeTooHigh,
            })?;

        Ok(Signature { sig })
    }

    #[inline(always)]
    pub fn verify_combined_signature(
        &self,
        msg: &[u8],
        sig: &Signature,
    ) -> Result<(), VerifySignatureError> {
        if self.public_key().verify_combined_signatures(sig, msg) {
            Ok(())
        } else {
            Err(VerifySignatureError::WrongSignature)
        }
    }
}

impl From<Commitment> for PublicKeySet {
    fn from(value: Commitment) -> Self {
        PublicKeySet {
            pk_set: threshold_crypto::PublicKeySet::from(value),
        }
    }
}

impl From<PublicKeySet> for super::PublicKeySet {
    fn from(value: PublicKeySet) -> Self {
        super::PublicKeySet { key: value }
    }
}

impl From<PrivateKeyPart> for super::PrivateKeyPart {
    fn from(value: PrivateKeyPart) -> Self {
        super::PrivateKeyPart { key: value }
    }
}

impl From<PrivateKeyPart> for SerializableKeyPart {
    fn from(value: PrivateKeyPart) -> Self {
        SerializableKeyPart {
            key: SerdeSecret(value.key),
        }
    }
}

impl From<SerializableKeyPart> for PrivateKeyPart {
    fn from(value: SerializableKeyPart) -> Self {
        PrivateKeyPart { key: value.key.0 }
    }
}

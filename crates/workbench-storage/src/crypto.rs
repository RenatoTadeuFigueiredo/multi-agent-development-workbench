use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::{Aead, KeyInit, Payload},
};
use rand::Rng;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use zeroize::{Zeroize, Zeroizing};

use crate::StorageError;

pub const KEY_BYTES: usize = 32;
pub const NONCE_BYTES: usize = 24;

/// A zeroizing 256-bit encryption key.
#[derive(Clone)]
pub struct SecretKey(Zeroizing<[u8; KEY_BYTES]>);

impl SecretKey {
    pub fn generate() -> Self {
        let mut bytes = [0_u8; KEY_BYTES];
        rand::rng().fill(&mut bytes);
        Self(Zeroizing::new(bytes))
    }

    pub fn from_slice(bytes: &[u8]) -> Result<Self, StorageError> {
        let bytes: [u8; KEY_BYTES] = bytes
            .try_into()
            .map_err(|_| StorageError::InvalidInput("key must be 256 bits"))?;
        Ok(Self(Zeroizing::new(bytes)))
    }

    pub(crate) fn expose(&self) -> &[u8; KEY_BYTES] {
        &self.0
    }
}

impl std::fmt::Debug for SecretKey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SecretKey([REDACTED])")
    }
}

/// Metadata authenticated with one encrypted event or key envelope.
#[derive(Debug, Clone, Serialize)]
pub struct AssociatedData<'a> {
    pub schema_version: u32,
    pub session_id: Uuid,
    pub object_id: &'a str,
    pub sequence: u64,
    pub kind: &'a str,
}

/// XChaCha20-Poly1305 output stored alongside non-sensitive ordering data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EncryptedPayload {
    pub nonce: Vec<u8>,
    pub ciphertext: Vec<u8>,
}

impl EncryptedPayload {
    pub fn encrypt(
        key: &SecretKey,
        plaintext: &[u8],
        associated_data: &AssociatedData<'_>,
    ) -> Result<Self, StorageError> {
        let cipher = XChaCha20Poly1305::new(key.expose().into());
        let mut nonce = [0_u8; NONCE_BYTES];
        rand::rng().fill(&mut nonce);
        let nonce_array = XNonce::from(nonce);
        let aad = serde_json::to_vec(associated_data)?;
        let ciphertext = cipher
            .encrypt(
                &nonce_array,
                Payload {
                    msg: plaintext,
                    aad: &aad,
                },
            )
            .map_err(|_| StorageError::AuthenticationFailed)?;
        Ok(Self {
            nonce: nonce.to_vec(),
            ciphertext,
        })
    }

    pub fn decrypt(
        &self,
        key: &SecretKey,
        associated_data: &AssociatedData<'_>,
    ) -> Result<Zeroizing<Vec<u8>>, StorageError> {
        let nonce: [u8; NONCE_BYTES] = self
            .nonce
            .as_slice()
            .try_into()
            .map_err(|_| StorageError::AuthenticationFailed)?;
        let nonce_array = XNonce::from(nonce);
        let cipher = XChaCha20Poly1305::new(key.expose().into());
        let aad = serde_json::to_vec(associated_data)?;
        let plaintext = cipher
            .decrypt(
                &nonce_array,
                Payload {
                    msg: &self.ciphertext,
                    aad: &aad,
                },
            )
            .map_err(|_| StorageError::AuthenticationFailed)?;
        Ok(Zeroizing::new(plaintext))
    }
}

impl Drop for SecretKey {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

#[cfg(test)]
mod tests {
    use super::{AssociatedData, EncryptedPayload, SecretKey};
    use uuid::Uuid;

    fn aad(session_id: Uuid, object_id: &str) -> AssociatedData<'_> {
        AssociatedData {
            schema_version: 1,
            session_id,
            object_id,
            sequence: 7,
            kind: "input_recorded",
        }
    }

    #[test]
    fn round_trip_and_fresh_nonce() {
        let key = SecretKey::generate();
        let session_id = Uuid::now_v7();
        let first = EncryptedPayload::encrypt(&key, b"sensitive", &aad(session_id, "event"))
            .expect("encrypt");
        let second = EncryptedPayload::encrypt(&key, b"sensitive", &aad(session_id, "event"))
            .expect("encrypt");

        assert_ne!(first.nonce, second.nonce);
        assert_eq!(
            first
                .decrypt(&key, &aad(session_id, "event"))
                .expect("decrypt")
                .as_slice(),
            b"sensitive"
        );
    }

    #[test]
    fn modified_associated_data_is_rejected() {
        let key = SecretKey::generate();
        let session_id = Uuid::now_v7();
        let encrypted = EncryptedPayload::encrypt(&key, b"sensitive", &aad(session_id, "event"))
            .expect("encrypt");

        assert!(
            encrypted
                .decrypt(&key, &aad(session_id, "different"))
                .is_err()
        );
        let different_session = Uuid::now_v7();
        assert!(
            encrypted
                .decrypt(&key, &aad(different_session, "event"))
                .is_err()
        );
        let mut different_sequence = aad(session_id, "event");
        different_sequence.sequence += 1;
        assert!(encrypted.decrypt(&key, &different_sequence).is_err());
        let mut different_kind = aad(session_id, "event");
        different_kind.kind = "provider_event";
        assert!(encrypted.decrypt(&key, &different_kind).is_err());
    }
}

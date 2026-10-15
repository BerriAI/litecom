use crate::error::{Error, Result};
use aes_gcm::{
    Aes256Gcm, KeyInit, Nonce,
    aead::{Aead, Payload},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use hmac::{Hmac, Mac};
use rand::{RngCore, rngs::OsRng};
use serde::{Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use zeroize::{Zeroize, ZeroizeOnDrop};

pub fn random_bytes<const N: usize>() -> [u8; N] {
    let mut bytes = [0; N];
    OsRng.fill_bytes(&mut bytes);
    bytes
}
pub fn secret() -> String {
    URL_SAFE_NO_PAD.encode(random_bytes::<32>())
}
pub fn hash(bytes: impl AsRef<[u8]>) -> String {
    hex::encode(Sha256::digest(bytes.as_ref()))
}
pub fn equal(a: &str, b: &str) -> bool {
    bool::from(a.as_bytes().ct_eq(b.as_bytes()))
}
pub fn id() -> String {
    uuid::Uuid::new_v4().to_string()
}

#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct Vault {
    key: [u8; 32],
}

impl Vault {
    pub fn new(key: [u8; 32]) -> Self {
        Self { key }
    }

    // A keyed fingerprint makes phone uniqueness independent of randomized ciphertext
    // without exposing a dictionary of unhashed phone numbers in the database.
    pub fn fingerprint(&self, context: &str, bytes: &[u8]) -> String {
        let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(&self.key).expect("32-byte key");
        mac.update(context.as_bytes());
        mac.update(&[0]);
        mac.update(bytes);
        hex::encode(mac.finalize().into_bytes())
    }

    pub fn encrypt(&self, bytes: &[u8], context: &str) -> Result<Vec<u8>> {
        let cipher = Aes256Gcm::new_from_slice(&self.key).expect("32-byte key");
        let nonce = random_bytes::<12>();
        let encrypted = cipher
            .encrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: bytes,
                    aad: context.as_bytes(),
                },
            )
            .map_err(|_| Error::Internal(anyhow::anyhow!("encryption failed")))?;
        let mut output = vec![1];
        output.extend(nonce);
        output.extend(encrypted);
        Ok(output)
    }

    pub fn decrypt(&self, bytes: &[u8], context: &str) -> Result<Vec<u8>> {
        if bytes.len() < 29 || bytes[0] != 1 {
            return Err(Error::Internal(anyhow::anyhow!("invalid encrypted record")));
        }
        Aes256Gcm::new_from_slice(&self.key)
            .expect("32-byte key")
            .decrypt(
                Nonce::from_slice(&bytes[1..13]),
                Payload {
                    msg: &bytes[13..],
                    aad: context.as_bytes(),
                },
            )
            .map_err(|_| Error::Internal(anyhow::anyhow!("encrypted record authentication failed")))
    }
    pub fn seal<T: Serialize>(&self, value: &T, context: &str) -> Result<Vec<u8>> {
        self.encrypt(&serde_json::to_vec(value)?, context)
    }
    pub fn open<T: DeserializeOwned>(&self, value: &[u8], context: &str) -> Result<T> {
        Ok(serde_json::from_slice(&self.decrypt(value, context)?)?)
    }
}

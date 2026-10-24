//! Envelope for secrets stored in PostgreSQL (GitHub user tokens). AES-256-GCM with a random 96-bit nonce;
//! the associated data binds a ciphertext to its owner so rows cannot be swapped between users.
//!
//! Layout: `[version=1][nonce: 12 bytes][ciphertext || tag]`.

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine;
use rand::RngCore;
use sha2::{Digest, Sha256};

const VERSION: u8 = 1;
const NONCE_LEN: usize = 12;

#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    #[error("encryption key must be 32 bytes encoded as base64")]
    BadKey,
    #[error("ciphertext is malformed or was produced with a different key")]
    Decrypt,
}

#[derive(Clone)]
pub struct TokenCipher {
    cipher: Aes256Gcm,
}

impl std::fmt::Debug for TokenCipher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TokenCipher(<redacted>)")
    }
}

impl TokenCipher {
    pub fn from_base64(key_b64: &str) -> Result<Self, CryptoError> {
        let key = STANDARD.decode(key_b64.trim()).map_err(|_| CryptoError::BadKey)?;
        if key.len() != 32 {
            return Err(CryptoError::BadKey);
        }
        let cipher = Aes256Gcm::new_from_slice(&key).map_err(|_| CryptoError::BadKey)?;
        Ok(Self { cipher })
    }

    pub fn encrypt(&self, plaintext: &[u8], aad: &[u8]) -> Vec<u8> {
        let mut nonce = [0u8; NONCE_LEN];
        rand::thread_rng().fill_bytes(&mut nonce);
        let ct = self
            .cipher
            .encrypt(Nonce::from_slice(&nonce), Payload { msg: plaintext, aad })
            .expect("AES-GCM encryption cannot fail for in-memory buffers");
        let mut out = Vec::with_capacity(1 + NONCE_LEN + ct.len());
        out.push(VERSION);
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&ct);
        out
    }

    pub fn decrypt(&self, data: &[u8], aad: &[u8]) -> Result<Vec<u8>, CryptoError> {
        if data.len() < 1 + NONCE_LEN + 16 || data[0] != VERSION {
            return Err(CryptoError::Decrypt);
        }
        let (nonce, ct) = data[1..].split_at(NONCE_LEN);
        self.cipher.decrypt(Nonce::from_slice(nonce), Payload { msg: ct, aad }).map_err(|_| CryptoError::Decrypt)
    }

    pub fn encrypt_str(&self, plaintext: &str, aad: &[u8]) -> Vec<u8> {
        self.encrypt(plaintext.as_bytes(), aad)
    }

    pub fn decrypt_string(&self, data: &[u8], aad: &[u8]) -> Result<String, CryptoError> {
        String::from_utf8(self.decrypt(data, aad)?).map_err(|_| CryptoError::Decrypt)
    }

    /// Encrypts and base64url-encodes, for short-lived cookies (OAuth state).
    pub fn seal_url_safe(&self, plaintext: &[u8], aad: &[u8]) -> String {
        URL_SAFE_NO_PAD.encode(self.encrypt(plaintext, aad))
    }

    pub fn open_url_safe(&self, sealed: &str, aad: &[u8]) -> Result<Vec<u8>, CryptoError> {
        let raw = URL_SAFE_NO_PAD.decode(sealed).map_err(|_| CryptoError::Decrypt)?;
        self.decrypt(&raw, aad)
    }
}

/// Cryptographically random URL-safe token with `bytes` bytes of entropy.
pub fn random_token(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    rand::thread_rng().fill_bytes(&mut buf);
    URL_SAFE_NO_PAD.encode(buf)
}

pub fn sha256(data: &[u8]) -> [u8; 32] {
    Sha256::digest(data).into()
}

pub fn sha256_hex(data: &[u8]) -> String {
    let digest = sha256(data);
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// PKCE S256 code challenge for a verifier.
pub fn pkce_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(sha256(verifier.as_bytes()))
}

/// Constant-time equality for secrets of equal length.
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> String {
        STANDARD.encode([7u8; 32])
    }

    #[test]
    fn round_trip_and_aad_binding() {
        let c = TokenCipher::from_base64(&key()).unwrap();
        let ct = c.encrypt_str("ghu_secret", b"user-1");
        assert_ne!(&ct[13..], b"ghu_secret");
        assert_eq!(c.decrypt_string(&ct, b"user-1").unwrap(), "ghu_secret");
        assert!(c.decrypt_string(&ct, b"user-2").is_err(), "AAD must bind the owner");
    }

    #[test]
    fn nonces_are_unique() {
        let c = TokenCipher::from_base64(&key()).unwrap();
        assert_ne!(c.encrypt(b"x", b""), c.encrypt(b"x", b""));
    }

    #[test]
    fn wrong_key_and_tampering_fail() {
        let c1 = TokenCipher::from_base64(&key()).unwrap();
        let c2 = TokenCipher::from_base64(&STANDARD.encode([8u8; 32])).unwrap();
        let mut ct = c1.encrypt(b"secret", b"");
        assert!(c2.decrypt(&ct, b"").is_err());
        let last = ct.len() - 1;
        ct[last] ^= 1;
        assert!(c1.decrypt(&ct, b"").is_err());
    }

    #[test]
    fn rejects_bad_keys() {
        assert!(TokenCipher::from_base64("short").is_err());
        assert!(TokenCipher::from_base64(&STANDARD.encode([1u8; 16])).is_err());
    }

    #[test]
    fn pkce_matches_rfc7636_example() {
        assert_eq!(
            pkce_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn constant_time() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
    }
}

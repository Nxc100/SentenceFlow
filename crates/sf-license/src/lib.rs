//! # sf-license — offline licensing (spec §7.6)
//!
//! A `.sflic` license is a small JSON document: the payload fields plus an
//! Ed25519 signature over the canonical payload bytes. The client embeds only
//! the *public* key and verifies locally — no activation server, no device
//! binding (spec §1.3).
//!
//! Canonical bytes = `serde_json::to_vec(&LicensePayload)`. Rust struct field
//! order is stable, so this is deterministic; the issuer and verifier both use
//! this crate, so the two sides can never drift apart.
//!
//! The signing side lives behind the `issuer` feature and is **never** enabled
//! in shipped binaries.

pub mod trial;

#[cfg(feature = "issuer")]
pub mod issuer;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const PRODUCT_ID: &str = "sentenceflow";
pub const FORMAT_VERSION: u32 = 1;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum LicenseError {
    #[error("许可证格式无效：请确认复制完整")]
    Malformed,
    #[error("签名无效：请确认复制完整")]
    BadSignature,
    #[error("这不是本产品的许可证")]
    WrongProduct,
    #[error("许可证版本过新，请升级软件后重试")]
    UnsupportedVersion,
}

/// The signed payload (spec §7.6). Field order is the canonical signing order
/// — do not reorder fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LicensePayload {
    pub v: u32,
    pub product: String,
    pub edition: String,
    pub email: String,
    /// Highest product major version this license covers (买断 + 大版本升级付费).
    pub major_max: u32,
    /// Unix seconds at issue time.
    pub issued_at: i64,
}

impl LicensePayload {
    pub fn canonical_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("payload serialization cannot fail")
    }

    /// Does this license cover product major version `major`?
    pub fn permits_major(&self, major: u32) -> bool {
        major <= self.major_max
    }

    /// `u***@example.com` — shown on the activated license card (§5.5).
    pub fn masked_email(&self) -> String {
        match self.email.split_once('@') {
            Some((local, domain)) if !local.is_empty() => {
                let first = local.chars().next().unwrap();
                format!("{first}***@{domain}")
            }
            _ => "***".to_string(),
        }
    }
}

/// On-disk / on-clipboard `.sflic` document: payload fields + base64 signature.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LicenseFile {
    #[serde(flatten)]
    pub payload: LicensePayload,
    pub sig: String,
}

/// Parse a base64-encoded 32-byte Ed25519 public key (the app's embedded key).
pub fn parse_public_key(b64: &str) -> Result<VerifyingKey, LicenseError> {
    let bytes = B64
        .decode(b64.trim())
        .map_err(|_| LicenseError::Malformed)?;
    let arr: [u8; 32] = bytes.try_into().map_err(|_| LicenseError::Malformed)?;
    VerifyingKey::from_bytes(&arr).map_err(|_| LicenseError::Malformed)
}

/// Verify a pasted/dropped `.sflic` document. Returns the trusted payload.
///
/// Verification is pure and fast (<50ms budget in §6.3; actual cost is ~50µs).
pub fn verify(sflic_json: &str, public_key: &VerifyingKey) -> Result<LicensePayload, LicenseError> {
    let file: LicenseFile =
        serde_json::from_str(sflic_json.trim()).map_err(|_| LicenseError::Malformed)?;
    if file.payload.product != PRODUCT_ID {
        return Err(LicenseError::WrongProduct);
    }
    if file.payload.v > FORMAT_VERSION {
        return Err(LicenseError::UnsupportedVersion);
    }
    let sig_bytes = B64.decode(&file.sig).map_err(|_| LicenseError::Malformed)?;
    let sig = Signature::from_slice(&sig_bytes).map_err(|_| LicenseError::BadSignature)?;
    public_key
        .verify(&file.payload.canonical_bytes(), &sig)
        .map_err(|_| LicenseError::BadSignature)?;
    Ok(file.payload)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    fn keypair() -> (SigningKey, VerifyingKey) {
        // Deterministic test key — NOT a real product key.
        let sk = SigningKey::from_bytes(&[7u8; 32]);
        let vk = sk.verifying_key();
        (sk, vk)
    }

    fn issue_for_test(sk: &SigningKey, payload: &LicensePayload) -> String {
        let sig = sk.sign(&payload.canonical_bytes());
        serde_json::to_string_pretty(&LicenseFile {
            payload: payload.clone(),
            sig: B64.encode(sig.to_bytes()),
        })
        .unwrap()
    }

    fn payload() -> LicensePayload {
        LicensePayload {
            v: 1,
            product: PRODUCT_ID.into(),
            edition: "personal".into(),
            email: "user@example.com".into(),
            major_max: 3,
            issued_at: 1_765_900_800,
        }
    }

    #[test]
    fn valid_license_verifies() {
        let (sk, vk) = keypair();
        let doc = issue_for_test(&sk, &payload());
        let p = verify(&doc, &vk).unwrap();
        assert_eq!(p.email, "user@example.com");
        assert!(p.permits_major(3));
        assert!(!p.permits_major(4));
    }

    #[test]
    fn tampered_payload_rejected() {
        let (sk, vk) = keypair();
        let doc = issue_for_test(&sk, &payload()).replace("personal", "ultimate");
        assert_eq!(verify(&doc, &vk), Err(LicenseError::BadSignature));
    }

    #[test]
    fn truncated_document_rejected() {
        let (sk, vk) = keypair();
        let doc = issue_for_test(&sk, &payload());
        let cut = &doc[..doc.len() - 10];
        assert_eq!(verify(cut, &vk), Err(LicenseError::Malformed));
    }

    #[test]
    fn wrong_key_rejected() {
        let (sk, _) = keypair();
        let other = SigningKey::from_bytes(&[9u8; 32]).verifying_key();
        let doc = issue_for_test(&sk, &payload());
        assert_eq!(verify(&doc, &other), Err(LicenseError::BadSignature));
    }

    #[test]
    fn wrong_product_rejected() {
        let (sk, vk) = keypair();
        let mut p = payload();
        p.product = "otherapp".into();
        let doc = issue_for_test(&sk, &p);
        assert_eq!(verify(&doc, &vk), Err(LicenseError::WrongProduct));
    }

    #[test]
    fn email_masking() {
        assert_eq!(payload().masked_email(), "u***@example.com");
    }

    #[test]
    fn public_key_roundtrip() {
        let (_, vk) = keypair();
        let b64 = B64.encode(vk.to_bytes());
        assert_eq!(parse_public_key(&b64).unwrap(), vk);
        assert!(parse_public_key("not base64!!").is_err());
    }
}

//! Issuing side (vendor only, `issuer` feature).
//!
//! The private key lives on an offline USB stick + password manager and never
//! enters the repository (spec §7.6). This module only handles bytes it is
//! given.

use crate::{LicenseFile, LicensePayload};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use ed25519_dalek::{Signer, SigningKey, VerifyingKey};

/// Generate a fresh keypair. Returns `(private_b64, public_b64)`.
pub fn generate_keypair() -> (String, String) {
    let mut rng = rand::rngs::OsRng;
    let sk = SigningKey::generate(&mut rng);
    (
        B64.encode(sk.to_bytes()),
        B64.encode(sk.verifying_key().to_bytes()),
    )
}

pub fn parse_private_key(b64: &str) -> Result<SigningKey, String> {
    let bytes = B64
        .decode(b64.trim())
        .map_err(|e| format!("私钥不是有效 base64: {e}"))?;
    let arr: [u8; 32] = bytes
        .try_into()
        .map_err(|_| "私钥长度必须是 32 字节".to_string())?;
    Ok(SigningKey::from_bytes(&arr))
}

pub fn public_key_of(sk: &SigningKey) -> VerifyingKey {
    sk.verifying_key()
}

/// Sign a payload into the final `.sflic` JSON document.
pub fn issue(payload: &LicensePayload, sk: &SigningKey) -> String {
    let sig = sk.sign(&payload.canonical_bytes());
    serde_json::to_string_pretty(&LicenseFile {
        payload: payload.clone(),
        sig: B64.encode(sig.to_bytes()),
    })
    .expect("license serialization cannot fail")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PRODUCT_ID, verify};

    #[test]
    fn issued_license_verifies_with_matching_public_key() {
        let (sk_b64, pk_b64) = generate_keypair();
        let sk = parse_private_key(&sk_b64).unwrap();
        let pk = crate::parse_public_key(&pk_b64).unwrap();
        let payload = LicensePayload {
            v: 1,
            product: PRODUCT_ID.into(),
            edition: "personal".into(),
            email: "a@b.co".into(),
            major_max: 3,
            issued_at: 1_765_900_800,
        };
        let doc = issue(&payload, &sk);
        assert_eq!(verify(&doc, &pk).unwrap(), payload);
    }
}

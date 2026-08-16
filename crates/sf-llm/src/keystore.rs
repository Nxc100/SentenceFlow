//! API-key storage in the OS keychain (spec §4.7, §7.5).
//!
//! Keys live in macOS Keychain / Windows 凭据管理器 / Secret Service; they are
//! held in memory as `SecretString` (zeroized on drop) and are **excluded**
//! from backups by construction — backup code has no path to this module.

use crate::types::ChannelId;
use secrecy::SecretString;
use thiserror::Error;

const SERVICE: &str = "sentenceflow";

#[derive(Debug, Error)]
pub enum KeystoreError {
    #[error("keychain unavailable: {0}")]
    Unavailable(String),
    #[error("no key stored")]
    NotFound,
}

fn entry(channel: ChannelId) -> Result<keyring::Entry, KeystoreError> {
    let account = match channel {
        ChannelId::Deepseek => "deepseek-api-key",
        ChannelId::Zen => "zen-api-key",
        // Local channels have no keys; a caller asking for one is a bug.
        ChannelId::Opencode | ChannelId::Ollama => {
            return Err(KeystoreError::Unavailable(
                "local channels store no keys".into(),
            ));
        }
    };
    keyring::Entry::new(SERVICE, account).map_err(|e| KeystoreError::Unavailable(e.to_string()))
}

/// Store (or replace) a channel's API key.
pub fn store_key(channel: ChannelId, key: &SecretString) -> Result<(), KeystoreError> {
    use secrecy::ExposeSecret;
    entry(channel)?
        .set_password(key.expose_secret())
        .map_err(|e| KeystoreError::Unavailable(e.to_string()))
}

/// Load a channel's API key, if configured.
pub fn load_key(channel: ChannelId) -> Result<SecretString, KeystoreError> {
    match entry(channel)?.get_password() {
        Ok(pw) => Ok(SecretString::from(pw)),
        Err(keyring::Error::NoEntry) => Err(KeystoreError::NotFound),
        Err(e) => Err(KeystoreError::Unavailable(e.to_string())),
    }
}

/// Remove a stored key (channel card "清除" action).
pub fn delete_key(channel: ChannelId) -> Result<(), KeystoreError> {
    match entry(channel)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(KeystoreError::Unavailable(e.to_string())),
    }
}

/// Masked display form for the key input (§5.5): `sk-••••••••xyz`.
pub fn mask_key(key: &str) -> String {
    let visible_tail: String = key
        .chars()
        .rev()
        .take(3)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let prefix: String = key.chars().take(3).collect();
    if key.len() <= 8 {
        "••••••••".into()
    } else {
        format!("{prefix}••••••••{visible_tail}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masking_never_reveals_middle() {
        let m = mask_key("sk-abcdef1234567890");
        assert!(m.starts_with("sk-"));
        assert!(m.ends_with("890"));
        assert!(!m.contains("abcdef"));
        assert_eq!(mask_key("short"), "••••••••");
    }

    #[test]
    fn local_channels_have_no_entries() {
        assert!(matches!(
            store_key(ChannelId::Opencode, &SecretString::from("x".to_string())),
            Err(KeystoreError::Unavailable(_))
        ));
    }
}

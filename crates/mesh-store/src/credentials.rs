use keyring::{Entry, Error as KeyringError};

use crate::{StoreError, StoreResult};

pub const HF_CREDENTIAL_SERVICE: &str = "mesh.model-provider.huggingface";
pub const HF_CREDENTIAL_ACCOUNT: &str = "default";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialLookup {
    Found,
    Missing,
    StoreUnavailable,
}

pub fn load_huggingface_token() -> StoreResult<Option<String>> {
    match entry()?.get_password() {
        Ok(token) => {
            let trimmed = token.trim().to_owned();
            if trimmed.is_empty() {
                Ok(None)
            } else {
                Ok(Some(trimmed))
            }
        }
        Err(KeyringError::NoEntry) => Ok(None),
        Err(error) => Err(map_keyring(error)),
    }
}

pub fn save_huggingface_token(token: &str) -> StoreResult<()> {
    let trimmed = token.trim();
    if trimmed.is_empty() {
        return Err(StoreError::Corrupt(
            "huggingface token must not be empty".to_owned(),
        ));
    }
    entry()?
        .set_password(trimmed)
        .map_err(map_keyring)
}

pub fn delete_huggingface_token() -> StoreResult<bool> {
    match entry()?.delete_credential() {
        Ok(()) => Ok(true),
        Err(KeyringError::NoEntry) => Ok(false),
        Err(error) => Err(map_keyring(error)),
    }
}

pub fn huggingface_token_lookup() -> CredentialLookup {
    match load_huggingface_token() {
        Ok(Some(_)) => CredentialLookup::Found,
        Ok(None) => CredentialLookup::Missing,
        Err(StoreError::CredentialStore(_)) => CredentialLookup::StoreUnavailable,
        Err(_) => CredentialLookup::StoreUnavailable,
    }
}

fn entry() -> StoreResult<Entry> {
    Entry::new(HF_CREDENTIAL_SERVICE, HF_CREDENTIAL_ACCOUNT).map_err(map_keyring)
}

fn map_keyring(error: KeyringError) -> StoreError {
    match error {
        KeyringError::NoEntry => StoreError::NotFound("huggingface token".to_owned()),
        other => StoreError::CredentialStore(other.to_string()),
    }
}

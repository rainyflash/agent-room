//! Shared credential location for Bridge and its local clients.

mod encrypted;

use std::{env, path::Path};

use keyring::{Entry, Error as KeyringError};

use encrypted::EncryptedStore;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretStoreFailure {
    Configuration,
    Unavailable,
    Corrupt,
}

enum Backend {
    Keyring,
    Encrypted(EncryptedStore),
}

/// Uses the OS keyring by default. An explicitly configured headless vault never
/// falls back to another identity store if its configuration or key is invalid.
pub struct LocalSecretStore {
    service: String,
    backend: Result<Backend, SecretStoreFailure>,
}

impl LocalSecretStore {
    pub fn from_environment(service: impl Into<String>) -> Self {
        let backend = match (
            env::var_os("AGENT_ROOM_BRIDGE_VAULT_DIR"),
            env::var_os("AGENT_ROOM_BRIDGE_VAULT_KEY_FILE"),
        ) {
            (None, None) => Ok(Backend::Keyring),
            (Some(directory), Some(key)) => {
                EncryptedStore::new(Path::new(&directory), Path::new(&key)).map(Backend::Encrypted)
            }
            _ => Err(SecretStoreFailure::Configuration),
        };
        Self {
            service: service.into(),
            backend,
        }
    }

    /// Opens an explicitly selected encrypted store, independently of environment.
    ///
    /// # Errors
    /// Returns a failure for an unsafe location, invalid key or unavailable storage.
    pub fn encrypted(
        service: impl Into<String>,
        directory: &Path,
        key_file: &Path,
    ) -> Result<Self, SecretStoreFailure> {
        Ok(Self {
            service: service.into(),
            backend: Ok(Backend::Encrypted(EncryptedStore::new(
                directory, key_file,
            )?)),
        })
    }

    /// # Errors
    /// Reports unavailable storage or damaged credentials; absence is `None`.
    pub fn read(&self, account: &str) -> Result<Option<String>, SecretStoreFailure> {
        self.validate_account(account)?;
        match self.backend.as_ref().map_err(|failure| *failure)? {
            Backend::Encrypted(store) => store.read(&self.service, account),
            Backend::Keyring => match self.entry(account)?.get_password() {
                Ok(value) => Ok(Some(value)),
                Err(KeyringError::NoEntry) => Ok(None),
                Err(_) => Err(SecretStoreFailure::Unavailable),
            },
        }
    }

    /// # Errors
    /// Reports invalid input, key or storage failures. Never writes plaintext files.
    pub fn write(&self, account: &str, value: &str) -> Result<(), SecretStoreFailure> {
        self.validate_account(account)?;
        if value.len() > 65_536 {
            return Err(SecretStoreFailure::Configuration);
        }
        match self.backend.as_ref().map_err(|failure| *failure)? {
            Backend::Encrypted(store) => store.write(&self.service, account, value),
            Backend::Keyring => self
                .entry(account)?
                .set_password(value)
                .map_err(|_| SecretStoreFailure::Unavailable),
        }
    }

    /// # Errors
    /// Reports storage failures. Deleting an absent account is idempotent.
    pub fn delete(&self, account: &str) -> Result<(), SecretStoreFailure> {
        self.validate_account(account)?;
        match self.backend.as_ref().map_err(|failure| *failure)? {
            Backend::Encrypted(store) => store.delete(&self.service, account),
            Backend::Keyring => match self.entry(account)?.delete_credential() {
                Ok(()) | Err(KeyringError::NoEntry) => Ok(()),
                Err(_) => Err(SecretStoreFailure::Unavailable),
            },
        }
    }

    fn entry(&self, account: &str) -> Result<Entry, SecretStoreFailure> {
        Entry::new(&self.service, account).map_err(|_| SecretStoreFailure::Unavailable)
    }

    fn validate_account(&self, account: &str) -> Result<(), SecretStoreFailure> {
        if [self.service.as_str(), account].into_iter().any(|value| {
            value.is_empty() || value.len() > 256 || value.chars().any(char::is_control)
        }) {
            return Err(SecretStoreFailure::Configuration);
        }
        Ok(())
    }
}

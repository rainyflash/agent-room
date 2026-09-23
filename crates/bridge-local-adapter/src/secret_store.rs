//! Shared credential location for Bridge and its local clients.

mod encrypted;
mod system;

use std::{env, path::Path};

use encrypted::EncryptedStore;
pub use system::SystemCredentialStore;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretStoreFailure {
    Configuration,
    Unavailable,
    Corrupt,
}

enum Backend {
    System(SystemCredentialStore),
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
        let service = service.into();
        let backend = match (
            env::var_os("AGENT_ROOM_BRIDGE_VAULT_DIR"),
            env::var_os("AGENT_ROOM_BRIDGE_VAULT_KEY_FILE"),
        ) {
            (None, None) => Ok(Backend::System(SystemCredentialStore::new(service.clone()))),
            (Some(directory), Some(key)) => {
                EncryptedStore::new(Path::new(&directory), Path::new(&key)).map(Backend::Encrypted)
            }
            _ => Err(SecretStoreFailure::Configuration),
        };
        Self { service, backend }
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
            Backend::System(store) => store.read(account),
        }
    }

    /// # Errors
    /// Reports invalid input, key or storage failures. Never writes plaintext files.
    pub fn write(&self, account: &str, value: &str) -> Result<(), SecretStoreFailure> {
        self.validate_value(account, value)?;
        match self.backend.as_ref().map_err(|failure| *failure)? {
            Backend::Encrypted(store) => store.write(&self.service, account, value),
            Backend::System(store) => store.write(account, value),
        }
    }

    /// Writes a value that the desktop, MCP and CLI installed next to this program also read.
    /// The macOS keychain then lets them read it without asking for the login password; see
    /// [`SystemCredentialStore::write_shared`]. Other stores do not tell programs apart.
    ///
    /// # Errors
    /// Reports invalid input, key or storage failures. Never writes plaintext files.
    pub fn write_shared(&self, account: &str, value: &str) -> Result<(), SecretStoreFailure> {
        self.validate_value(account, value)?;
        match self.backend.as_ref().map_err(|failure| *failure)? {
            Backend::Encrypted(store) => store.write(&self.service, account, value),
            Backend::System(store) => store.write_shared(account, value),
        }
    }

    /// # Errors
    /// Reports storage failures. Deleting an absent account is idempotent.
    pub fn delete(&self, account: &str) -> Result<(), SecretStoreFailure> {
        self.validate_account(account)?;
        match self.backend.as_ref().map_err(|failure| *failure)? {
            Backend::Encrypted(store) => store.delete(&self.service, account),
            Backend::System(store) => store.delete(account),
        }
    }

    fn validate_value(&self, account: &str, value: &str) -> Result<(), SecretStoreFailure> {
        self.validate_account(account)?;
        if value.len() > 65_536 {
            return Err(SecretStoreFailure::Configuration);
        }
        Ok(())
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

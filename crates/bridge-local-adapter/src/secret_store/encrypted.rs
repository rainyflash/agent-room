use std::{
    fs::{self, File},
    io::{Read as _, Write as _},
    path::{Component, Path, PathBuf},
};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chacha20poly1305::{
    Key, KeyInit as _, XChaCha20Poly1305, XNonce,
    aead::{Aead as _, Payload},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use zeroize::Zeroizing;

use super::SecretStoreFailure;

pub(super) struct EncryptedStore {
    directory: PathBuf,
    key: Zeroizing<[u8; 32]>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    version: u8,
    nonce: String,
    ciphertext: String,
}

impl EncryptedStore {
    pub(super) fn new(directory: &Path, key_file: &Path) -> Result<Self, SecretStoreFailure> {
        for path in [directory, key_file] {
            if !path.is_absolute() || path.components().any(|part| part == Component::ParentDir) {
                return Err(SecretStoreFailure::Configuration);
            }
        }
        let key = Zeroizing::new(read_private_file(key_file, 32)?);
        let bytes: [u8; 32] = key
            .as_slice()
            .try_into()
            .map_err(|_| SecretStoreFailure::Configuration)?;
        if !directory.exists() {
            let mut builder = fs::DirBuilder::new();
            builder.recursive(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt as _;
                builder.mode(0o700);
            }
            builder
                .create(directory)
                .map_err(|_| SecretStoreFailure::Unavailable)?;
        }
        let metadata = private_metadata(directory)?;
        if !metadata.is_dir() {
            return Err(SecretStoreFailure::Configuration);
        }
        Ok(Self {
            directory: directory.to_owned(),
            key: Zeroizing::new(bytes),
        })
    }

    pub(super) fn read(
        &self,
        service: &str,
        account: &str,
    ) -> Result<Option<String>, SecretStoreFailure> {
        let path = self.path(service, account);
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(SecretStoreFailure::Unavailable),
            Ok(_) => {}
        }
        let envelope: Envelope = serde_json::from_slice(&read_private_file(&path, 131_072)?)
            .map_err(|_| SecretStoreFailure::Corrupt)?;
        if envelope.version != 1 {
            return Err(SecretStoreFailure::Corrupt);
        }
        let nonce: [u8; 24] = URL_SAFE_NO_PAD
            .decode(envelope.nonce)
            .map_err(|_| SecretStoreFailure::Corrupt)?
            .try_into()
            .map_err(|_| SecretStoreFailure::Corrupt)?;
        let ciphertext = URL_SAFE_NO_PAD
            .decode(envelope.ciphertext)
            .map_err(|_| SecretStoreFailure::Corrupt)?;
        let plaintext = self
            .cipher()
            .decrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: &ciphertext,
                    aad: &context(service, account),
                },
            )
            .map_err(|_| SecretStoreFailure::Corrupt)?;
        String::from_utf8(plaintext)
            .map(Some)
            .map_err(|_| SecretStoreFailure::Corrupt)
    }

    pub(super) fn write(
        &self,
        service: &str,
        account: &str,
        value: &str,
    ) -> Result<(), SecretStoreFailure> {
        let mut nonce = [0_u8; 24];
        getrandom::fill(&mut nonce).map_err(|_| SecretStoreFailure::Unavailable)?;
        let ciphertext = self
            .cipher()
            .encrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: value.as_bytes(),
                    aad: &context(service, account),
                },
            )
            .map_err(|_| SecretStoreFailure::Unavailable)?;
        let envelope = Envelope {
            version: 1,
            nonce: URL_SAFE_NO_PAD.encode(nonce),
            ciphertext: URL_SAFE_NO_PAD.encode(ciphertext),
        };
        let bytes = serde_json::to_vec(&envelope).map_err(|_| SecretStoreFailure::Corrupt)?;
        let mut file = tempfile::NamedTempFile::new_in(&self.directory)
            .map_err(|_| SecretStoreFailure::Unavailable)?;
        // NamedTempFile creates owner-only files on Unix; verify before persisting.
        private_metadata(file.path())?;
        file.write_all(&bytes)
            .map_err(|_| SecretStoreFailure::Unavailable)?;
        file.as_file()
            .sync_all()
            .map_err(|_| SecretStoreFailure::Unavailable)?;
        file.persist(self.path(service, account))
            .map_err(|_| SecretStoreFailure::Unavailable)?;
        #[cfg(unix)]
        self.sync_directory()?;
        Ok(())
    }

    pub(super) fn delete(&self, service: &str, account: &str) -> Result<(), SecretStoreFailure> {
        match fs::remove_file(self.path(service, account)) {
            Ok(()) => {
                #[cfg(unix)]
                self.sync_directory()?;
                Ok(())
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err(SecretStoreFailure::Unavailable),
        }
    }

    fn path(&self, service: &str, account: &str) -> PathBuf {
        self.directory.join(format!(
            "{}.json",
            URL_SAFE_NO_PAD.encode(Sha256::digest(context(service, account)))
        ))
    }

    fn cipher(&self) -> XChaCha20Poly1305 {
        XChaCha20Poly1305::new(Key::from_slice(self.key.as_ref()))
    }

    #[cfg(unix)]
    fn sync_directory(&self) -> Result<(), SecretStoreFailure> {
        File::open(&self.directory)
            .and_then(|file| file.sync_all())
            .map_err(|_| SecretStoreFailure::Unavailable)?;
        Ok(())
    }
}

fn context(service: &str, account: &str) -> Vec<u8> {
    format!("agent-room-vault-v1\0{service}\0{account}").into_bytes()
}

fn private_metadata(path: &Path) -> Result<fs::Metadata, SecretStoreFailure> {
    let metadata = fs::symlink_metadata(path).map_err(|_| SecretStoreFailure::Unavailable)?;
    if metadata.file_type().is_symlink() {
        return Err(SecretStoreFailure::Configuration);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(SecretStoreFailure::Configuration);
        }
    }
    Ok(metadata)
}

fn read_private_file(path: &Path, maximum: usize) -> Result<Vec<u8>, SecretStoreFailure> {
    let metadata = private_metadata(path)?;
    if !metadata.is_file() || metadata.len() > maximum as u64 {
        return Err(SecretStoreFailure::Configuration);
    }
    let mut bytes = Vec::new();
    File::open(path)
        .and_then(|file| file.take(maximum as u64 + 1).read_to_end(&mut bytes))
        .map_err(|_| SecretStoreFailure::Unavailable)?;
    if bytes.len() > maximum {
        return Err(SecretStoreFailure::Configuration);
    }
    Ok(bytes)
}

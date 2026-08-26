use std::{fmt::Write as _, path::Path};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use ed25519_dalek::{SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use zeroize::Zeroize as _;

use crate::{
    error::{ToolError, ToolResult},
    files::read_text,
};

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PrivateKeyDocument {
    schema_version: u32,
    algorithm: String,
    key_id: String,
    private_key: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PublicKeyDocument {
    schema_version: u32,
    algorithm: String,
    key_id: String,
    public_key: String,
}

impl PrivateKeyDocument {
    pub fn from_signing_key(signing_key: &SigningKey) -> Self {
        Self {
            schema_version: 1,
            algorithm: "Ed25519".to_owned(),
            key_id: key_id(&signing_key.verifying_key()),
            private_key: URL_SAFE_NO_PAD.encode(signing_key.to_bytes()),
        }
    }

    pub fn key_id(&self) -> &str {
        &self.key_id
    }

    pub fn decode(mut self) -> ToolResult<SigningKey> {
        validate_document(self.schema_version, &self.algorithm)?;
        let decoded = URL_SAFE_NO_PAD
            .decode(&self.private_key)
            .map_err(|_| ToolError::InvalidKeyEncoding);
        self.private_key.zeroize();
        let mut decoded = decoded?;
        let bytes: [u8; 32] = decoded
            .as_slice()
            .try_into()
            .map_err(|_| ToolError::InvalidKeyLength)?;
        decoded.zeroize();
        let signing_key = SigningKey::from_bytes(&bytes);
        if key_id(&signing_key.verifying_key()) != self.key_id {
            return Err(ToolError::KeyIdMismatch);
        }
        Ok(signing_key)
    }
}

impl Drop for PrivateKeyDocument {
    fn drop(&mut self) {
        self.private_key.zeroize();
    }
}

impl PublicKeyDocument {
    pub fn from_verifying_key(verifying_key: &VerifyingKey) -> Self {
        Self {
            schema_version: 1,
            algorithm: "Ed25519".to_owned(),
            key_id: key_id(verifying_key),
            public_key: URL_SAFE_NO_PAD.encode(verifying_key.to_bytes()),
        }
    }

    pub fn decode(self) -> ToolResult<agent_room_release_manifest::TrustedReleaseKey> {
        validate_document(self.schema_version, &self.algorithm)?;
        let decoded = URL_SAFE_NO_PAD
            .decode(self.public_key)
            .map_err(|_| ToolError::InvalidKeyEncoding)?;
        let public_key: [u8; 32] = decoded
            .as_slice()
            .try_into()
            .map_err(|_| ToolError::InvalidKeyLength)?;
        let verifying_key =
            VerifyingKey::from_bytes(&public_key).map_err(|_| ToolError::InvalidKeyEncoding)?;
        if key_id(&verifying_key) != self.key_id {
            return Err(ToolError::KeyIdMismatch);
        }
        Ok(agent_room_release_manifest::TrustedReleaseKey {
            key_id: self.key_id,
            public_key,
        })
    }
}

pub fn read_private_key(path: &Path) -> ToolResult<(String, SigningKey)> {
    let mut text = read_text(path)?;
    let document = serde_json::from_str::<PrivateKeyDocument>(&text);
    text.zeroize();
    let document = document?;
    let key_id = document.key_id().to_owned();
    Ok((key_id, document.decode()?))
}

pub fn read_public_key(path: &Path) -> ToolResult<agent_room_release_manifest::TrustedReleaseKey> {
    let document: PublicKeyDocument = serde_json::from_str(&read_text(path)?)?;
    document.decode()
}

fn validate_document(schema_version: u32, algorithm: &str) -> ToolResult<()> {
    if schema_version != 1 || algorithm != "Ed25519" {
        return Err(ToolError::UnsupportedKeyDocument);
    }
    Ok(())
}

fn key_id(verifying_key: &VerifyingKey) -> String {
    let digest = Sha256::digest(verifying_key.as_bytes());
    let prefix = digest[..16]
        .iter()
        .fold(String::with_capacity(32), |mut output, byte| {
            write!(output, "{byte:02x}").expect("写入 String 不会失败");
            output
        });
    format!("ed25519-sha256-{prefix}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_documents_round_trip() {
        let signing_key = SigningKey::from_bytes(&[11; 32]);
        let private = PrivateKeyDocument::from_signing_key(&signing_key);
        let public = PublicKeyDocument::from_verifying_key(&signing_key.verifying_key());

        assert_eq!(private.key_id(), public.key_id);
        assert_eq!(private.decode().expect("私钥必须可回读"), signing_key);
        assert_eq!(
            public.decode().expect("公钥必须可回读").public_key,
            signing_key.verifying_key().to_bytes()
        );
    }
}

use std::{fs, time::SystemTime};

use agent_room_release_manifest::{
    ReleaseChannel, ReleaseManifest, ReleaseTrustState, SignedReleaseManifest,
    validate_release_document, verify_release,
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use ed25519_dalek::{Signer as _, SigningKey};
use zeroize::Zeroizing;

use crate::{
    cli::{ChannelArg, KeygenArgs, SignArgs, VerifyArgs},
    error::{ToolError, ToolResult},
    files::{read_text, write_new_json, write_new_private_json},
    keys::{PrivateKeyDocument, PublicKeyDocument, read_private_key, read_public_key},
};

pub fn keygen(args: &KeygenArgs) -> ToolResult<()> {
    if args.private_key.exists() {
        return Err(ToolError::RefuseOverwrite(args.private_key.clone()));
    }
    if args.public_key.exists() {
        return Err(ToolError::RefuseOverwrite(args.public_key.clone()));
    }

    let mut secret = Zeroizing::new([0_u8; 32]);
    getrandom::fill(secret.as_mut()).map_err(|_| ToolError::RandomSource)?;
    let signing_key = SigningKey::from_bytes(&secret);
    let private_document = PrivateKeyDocument::from_signing_key(&signing_key);
    let public_document = PublicKeyDocument::from_verifying_key(&signing_key.verifying_key());

    write_new_private_json(&args.private_key, &private_document)?;
    if let Err(error) = write_new_json(&args.public_key, &public_document) {
        let _ = fs::remove_file(&args.private_key);
        return Err(error);
    }
    println!("已生成发布密钥：{}", private_document.key_id());
    Ok(())
}

pub fn sign(args: &SignArgs) -> ToolResult<()> {
    let (key_id, signing_key) = read_private_key(&args.private_key)?;
    let manifest: ReleaseManifest = serde_json::from_str(&read_text(&args.manifest)?)?;
    validate_release_document(&manifest, now_unix_seconds()?)?;
    let payload = serde_json::to_vec(&manifest)?;
    let signature = signing_key.sign(&payload);
    let envelope = SignedReleaseManifest {
        algorithm: "Ed25519".to_owned(),
        key_id,
        payload: URL_SAFE_NO_PAD.encode(payload),
        signature: URL_SAFE_NO_PAD.encode(signature.to_bytes()),
    };
    write_new_json(&args.output, &envelope)?;
    println!("已写入签名发布清单：{}", args.output.display());
    Ok(())
}

pub fn verify(args: &VerifyArgs) -> ToolResult<()> {
    let trusted_key = read_public_key(&args.public_key)?;
    let envelope: SignedReleaseManifest = serde_json::from_str(&read_text(&args.manifest)?)?;
    let channel = match args.channel {
        ChannelArg::Stable => ReleaseChannel::Stable,
        ChannelArg::Testing => ReleaseChannel::Testing,
    };
    let state = ReleaseTrustState {
        channel,
        highest_sequence: args.highest_sequence,
        installed_version: args.installed_version.clone(),
    };
    let verified = verify_release(
        &envelope,
        &trusted_key,
        channel,
        &state,
        args.now_unix_seconds.map_or_else(now_unix_seconds, Ok)?,
    )?;
    println!("{}", serde_json::to_string_pretty(verified.manifest())?);
    Ok(())
}

fn now_unix_seconds() -> ToolResult<u64> {
    Ok(SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_secs())
}

#[cfg(test)]
mod tests {
    use agent_room_release_manifest::{ArtifactKind, ReleaseArtifact};
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn offline_signing_workflow_round_trips() {
        let directory = tempdir().expect("必须能创建测试目录");
        let private_key = directory.path().join("release-private.json");
        let public_key = directory.path().join("release-public.json");
        let manifest_path = directory.path().join("release.json");
        let signed_path = directory.path().join("release.signed.json");
        let now = now_unix_seconds().expect("测试时钟必须可用");

        keygen(&KeygenArgs {
            private_key: private_key.clone(),
            public_key: public_key.clone(),
        })
        .expect("必须能生成离线密钥");

        let manifest = ReleaseManifest {
            schema_version: 1,
            channel: ReleaseChannel::Stable,
            sequence: 1,
            version: "0.2.0".to_owned(),
            published_at_unix_seconds: now - 1,
            expires_at_unix_seconds: now + 3_600,
            rollback_from: None,
            tauri_manifest_url: None,
            artifacts: vec![ReleaseArtifact {
                kind: ArtifactKind::Bridge,
                platform: "windows-x86_64".to_owned(),
                url: "https://releases.example/bridge.exe".to_owned(),
                sha256: "b".repeat(64),
                byte_length: 64,
                sbom_url: "https://releases.example/bridge.cdx.json".to_owned(),
                signature_url: "https://releases.example/bridge.sig".to_owned(),
            }],
        };
        fs::write(
            &manifest_path,
            serde_json::to_vec_pretty(&manifest).expect("测试清单必须可序列化"),
        )
        .expect("必须能写入测试清单");

        sign(&SignArgs {
            private_key,
            manifest: manifest_path,
            output: signed_path.clone(),
        })
        .expect("必须能签名清单");
        verify(&VerifyArgs {
            public_key,
            manifest: signed_path,
            channel: ChannelArg::Stable,
            installed_version: "0.1.0".to_owned(),
            highest_sequence: 0,
            now_unix_seconds: Some(now),
        })
        .expect("必须能验证签名清单");
    }
}

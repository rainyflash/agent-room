use std::{fs, io::Write as _, path::PathBuf};

use agent_room_bridge_local_adapter::{LocalSecretStore, SecretStoreFailure};

fn key(root: &std::path::Path, bytes: &[u8]) -> PathBuf {
    let mut file = tempfile::NamedTempFile::new_in(root).unwrap();
    file.write_all(bytes).unwrap();
    file.keep().unwrap().1
}

#[test]
fn 重开仍可读取并且磁盘不保存明文且删除幂等() {
    let root = tempfile::tempdir().unwrap();
    let key_path = key(root.path(), &[7; 32]);
    let directory = root.path().join("vault");
    let store = LocalSecretStore::encrypted("test.owner", &directory, &key_path).unwrap();
    assert_eq!(store.read("session").unwrap(), None);
    store.write("session", "private-refresh-token").unwrap();
    let sealed_path = fs::read_dir(&directory)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let first = fs::read_to_string(&sealed_path).unwrap();
    assert!(!first.contains("private-refresh-token"));
    drop(store);
    let restored = LocalSecretStore::encrypted("test.owner", &directory, &key_path).unwrap();
    assert_eq!(
        restored.read("session").unwrap().as_deref(),
        Some("private-refresh-token")
    );
    restored.write("session", "private-refresh-token").unwrap();
    assert_ne!(fs::read_to_string(&sealed_path).unwrap(), first);
    restored.delete("session").unwrap();
    restored.delete("session").unwrap();
    assert_eq!(restored.read("session").unwrap(), None);
}

#[test]
fn 错误密钥损坏密文与不同命名空间均不能返回凭据() {
    let root = tempfile::tempdir().unwrap();
    let key_path = key(root.path(), &[7; 32]);
    let directory = root.path().join("vault");
    let store = LocalSecretStore::encrypted("test.owner", &directory, &key_path).unwrap();
    store.write("session", "secret").unwrap();
    let wrong_key = key(root.path(), &[8; 32]);
    let wrong = LocalSecretStore::encrypted("test.owner", &directory, &wrong_key).unwrap();
    assert_eq!(wrong.read("session"), Err(SecretStoreFailure::Corrupt));
    let other = LocalSecretStore::encrypted("other.owner", &directory, &key_path).unwrap();
    assert_eq!(other.read("session").unwrap(), None);
    let sealed_path = fs::read_dir(&directory)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    fs::write(sealed_path, b"{}").unwrap();
    assert_eq!(store.read("session"), Err(SecretStoreFailure::Corrupt));
}

#[test]
fn 密文绑定账户名称且无效密钥不会创建存储() {
    let root = tempfile::tempdir().unwrap();
    let key_path = key(root.path(), &[7; 32]);
    let bad_key = key(root.path(), &[7; 31]);
    let directory_a = root.path().join("a");
    assert!(LocalSecretStore::encrypted("owner", &directory_a, &bad_key).is_err());
    assert!(!directory_a.exists());
    let directory_b = root.path().join("b");
    let a = LocalSecretStore::encrypted("owner", &directory_a, &key_path).unwrap();
    let b = LocalSecretStore::encrypted("owner", &directory_b, &key_path).unwrap();
    a.write("account-a", "secret-a").unwrap();
    b.write("account-b", "secret-b").unwrap();
    let file_a = fs::read_dir(&directory_a)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let file_b = fs::read_dir(&directory_b)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    fs::copy(file_a, file_b).unwrap();
    assert_eq!(b.read("account-b"), Err(SecretStoreFailure::Corrupt));
}

#[cfg(unix)]
#[test]
fn 拒绝宽权限密钥与符号链接() {
    use std::os::unix::fs::{PermissionsExt as _, symlink};
    let root = tempfile::tempdir().unwrap();
    let key_path = key(root.path(), &[7; 32]);
    fs::set_permissions(&key_path, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(LocalSecretStore::encrypted("owner", &root.path().join("a"), &key_path).is_err());
    fs::set_permissions(&key_path, fs::Permissions::from_mode(0o600)).unwrap();
    let link = root.path().join("link");
    symlink(&key_path, &link).unwrap();
    assert!(LocalSecretStore::encrypted("owner", &root.path().join("a"), &link).is_err());
}

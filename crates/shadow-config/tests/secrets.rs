//! SecretStore 集成测试 -- ChaCha20-Poly1305 加密、密钥文件管理、前缀分派。

use shadow_config::{is_encrypted, SecretStore};

#[test]
fn encrypt_then_decrypt_round_trips() {
    let tmp = tempfile::tempdir().unwrap();
    let store = SecretStore::new(tmp.path(), true).unwrap();
    let plaintext = "sk-secret-api-key";
    let encrypted = store.encrypt(plaintext).unwrap();
    assert_ne!(encrypted, plaintext);
    let decrypted = store.decrypt(&encrypted).unwrap();
    assert_eq!(decrypted, plaintext);
}

#[test]
fn encrypted_value_has_enc2_prefix() {
    let tmp = tempfile::tempdir().unwrap();
    let store = SecretStore::new(tmp.path(), true).unwrap();
    let encrypted = store.encrypt("secret").unwrap();
    assert!(encrypted.starts_with("enc2:"));
}

#[test]
fn decrypt_passes_through_bare_value() {
    let tmp = tempfile::tempdir().unwrap();
    let store = SecretStore::new(tmp.path(), true).unwrap();
    // 裸值(明文)直接透传 -- 兼容现有明文配置
    let decrypted = store.decrypt("plain-text-key").unwrap();
    assert_eq!(decrypted, "plain-text-key");
}

#[test]
fn encrypt_empty_returns_empty() {
    let tmp = tempfile::tempdir().unwrap();
    let store = SecretStore::new(tmp.path(), true).unwrap();
    assert_eq!(store.encrypt("").unwrap(), "");
    assert_eq!(store.decrypt("").unwrap(), "");
}

#[test]
fn is_encrypted_classifies_correctly() {
    assert!(is_encrypted("enc2:abc123"));
    assert!(!is_encrypted("plain-text"));
    assert!(!is_encrypted(""));
}

#[cfg(unix)]
#[test]
fn key_file_has_restricted_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::tempdir().unwrap();
    let _store = SecretStore::new(tmp.path(), true).unwrap();
    let key_path = tmp.path().join(".secret_key");
    assert!(key_path.exists());
    let mode = std::fs::metadata(&key_path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
}

#[test]
fn new_store_reuses_existing_key() {
    let tmp = tempfile::tempdir().unwrap();
    let store1 = SecretStore::new(tmp.path(), true).unwrap();
    let encrypted = store1.encrypt("secret").unwrap();
    // 第二次 new 应该复用同一个密钥文件
    let store2 = SecretStore::new(tmp.path(), true).unwrap();
    let decrypted = store2.decrypt(&encrypted).unwrap();
    assert_eq!(decrypted, "secret");
}

#[test]
fn disabled_store_passes_through() {
    let tmp = tempfile::tempdir().unwrap();
    let store = SecretStore::new(tmp.path(), false).unwrap();
    // enabled=false 时 encrypt 是透传,不产生 enc2: 前缀
    let encrypted = store.encrypt("secret").unwrap();
    assert_eq!(encrypted, "secret");
    assert_eq!(store.decrypt("secret").unwrap(), "secret");
}

#[test]
fn decrypt_rejects_tampered_ciphertext() {
    let tmp = tempfile::tempdir().unwrap();
    let store = SecretStore::new(tmp.path(), true).unwrap();
    let encrypted = store.encrypt("secret").unwrap();
    // 篡改:翻转最后一个 hex 字符 -- AEAD 应检测到认证失败
    let mut tampered = encrypted.into_bytes();
    let last = tampered.len() - 1;
    tampered[last] = if tampered[last] == b'0' { b'1' } else { b'0' };
    let tampered = String::from_utf8(tampered).unwrap();
    assert!(store.decrypt(&tampered).is_err());
}

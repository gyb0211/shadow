//! 加密密钥存储 -- ChaCha20-Poly1305 AEAD
//!
//! 借鉴 zeroclaw-config 的 SecretStore,精简为最小可用形态:
//!
//! - 256 位密钥存于 `<shadow_dir>/.secret_key`(Unix 0600)
//! - 首次使用自动生成密钥
//! - 加密结果以 `enc2:` 前缀 + hex 编码存储
//! - 解密按前缀分派:`enc2:` 解密,裸值透传(兼容现有明文配置)
//!
//! 跳过的 zeroclaw 特性:`enc:` XOR 遗留迁移、`op://` 1Password CLI 集成、Windows ACL。

use anyhow::{anyhow, Result};
use chacha20poly1305::{
    aead::{Aead, KeyInit},
    ChaCha20Poly1305, Key, Nonce,
};
use rand::TryRngCore;
use std::path::Path;

const ENC_PREFIX: &str = "enc2:";
const KEY_FILE: &str = ".secret_key";
const KEY_LEN: usize = 32;
const NONCE_LEN: usize = 12;

/// ChaCha20-Poly1305 密钥存储。
pub struct SecretStore {
    cipher: ChaCha20Poly1305,
    enabled: bool,
}

impl SecretStore {
    /// 打开或创建密钥存储。`enabled = false` 时 encrypt/decrypt 均透传(明文模式)。
    pub fn new(shadow_dir: &Path, enabled: bool) -> Result<Self> {
        let key_bytes = load_or_create_key(shadow_dir)?;
        let cipher = ChaCha20Poly1305::new(Key::from_slice(&key_bytes));
        Ok(Self { cipher, enabled })
    }

    /// 加密明文。空串原样返回。`enabled = false` 时原样返回。
    /// 非空且启用时返回 `enc2:<hex(nonce || ciphertext || tag)>`。
    pub fn encrypt(&self, plaintext: &str) -> Result<String> {
        if !self.enabled || plaintext.is_empty() {
            return Ok(plaintext.to_string());
        }
        let nonce_bytes = random_bytes::<NONCE_LEN>();
        let nonce = Nonce::from_slice(&nonce_bytes);
        let ciphertext = self
            .cipher
            .encrypt(nonce, plaintext.as_bytes())
            .map_err(|e| anyhow!("encrypt failed: {e}"))?;
        let mut blob = nonce_bytes.to_vec();
        blob.extend_from_slice(&ciphertext);
        Ok(format!("{ENC_PREFIX}{}", hex::encode(&blob)))
    }

    /// 解密。空串返回空串。`enc2:` 前缀走 AEAD 解密;裸值原样返回(兼容明文配置)。
    pub fn decrypt(&self, value: &str) -> Result<String> {
        if value.is_empty() {
            return Ok(String::new());
        }
        if !self.enabled {
            return Ok(value.to_string());
        }
        let Some(hex_part) = value.strip_prefix(ENC_PREFIX) else {
            return Ok(value.to_string());
        };
        let blob = hex::decode(hex_part).map_err(|e| anyhow!("hex decode: {e}"))?;
        if blob.len() < NONCE_LEN {
            return Err(anyhow!("ciphertext too short"));
        }
        let (nonce_bytes, ciphertext) = blob.split_at(NONCE_LEN);
        let nonce = Nonce::from_slice(nonce_bytes);
        let plaintext = self
            .cipher
            .decrypt(nonce, ciphertext)
            .map_err(|e| anyhow!("decrypt failed: {e}"))?;
        String::from_utf8(plaintext).map_err(|e| anyhow!("utf8: {e}"))
    }
}

/// 判断值是否为加密形态(以 `enc2:` 开头)。
#[must_use]
pub fn is_encrypted(value: &str) -> bool {
    value.starts_with(ENC_PREFIX)
}

fn random_bytes<const N: usize>() -> [u8; N] {
    let mut arr = [0u8; N];
    // OsRng 失败意味着系统 RNG 损坏 -- 不可恢复,panic 合理
    rand::rngs::OsRng
        .try_fill_bytes(&mut arr)
        .expect("system RNG failed");
    arr
}

/// 读取或创建 256 位密钥文件。首次创建时设置 0600 权限(Unix)。
fn load_or_create_key(shadow_dir: &Path) -> Result<[u8; KEY_LEN]> {
    let key_path = shadow_dir.join(KEY_FILE);
    if key_path.exists() {
        let hex_str = std::fs::read_to_string(&key_path)?;
        let bytes = hex::decode(hex_str.trim()).map_err(|e| anyhow!("key hex decode: {e}"))?;
        if bytes.len() != KEY_LEN {
            return Err(anyhow!("key file wrong length: {}", bytes.len()));
        }
        let mut arr = [0u8; KEY_LEN];
        arr.copy_from_slice(&bytes);
        Ok(arr)
    } else {
        let key_bytes = random_bytes::<KEY_LEN>();
        std::fs::create_dir_all(shadow_dir)?;
        std::fs::write(&key_path, hex::encode(key_bytes))?;
        set_permissions_0600(&key_path)?;
        Ok(key_bytes)
    }
}

#[cfg(unix)]
fn set_permissions_0600(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .map_err(|e| anyhow!("set permissions: {e}"))
}

#[cfg(not(unix))]
fn set_permissions_0600(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
}

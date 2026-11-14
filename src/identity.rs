//! ed25519 身份。公钥即节点 ID，消息与握手都用它签名。

use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};

use crate::util::{now_ms, peer_id, short_id};

#[derive(Clone, Debug)]
pub struct Identity {
    pub signing: SigningKey,
    pub created_at: u64,
}

#[derive(Serialize, Deserialize)]
struct StoredIdentity {
    #[serde(default = "default_version")]
    version: u8,
    seed: String,
    #[serde(default)]
    nick: String,
    #[serde(default)]
    created_at: u64,
}

fn default_version() -> u8 {
    1
}

impl Identity {
    pub fn generate() -> Self {
        Self {
            signing: SigningKey::generate(&mut OsRng),
            created_at: now_ms(),
        }
    }

    /// 读取已有身份；文件不存在时生成并写入。
    /// 第二个返回值是文件里保存的昵称（如果有）。
    pub fn load_or_create(path: &Path) -> Result<(Self, Option<String>)> {
        if path.exists() {
            let text = std::fs::read_to_string(path)
                .with_context(|| format!("读取身份文件失败: {}", path.display()))?;
            let stored: StoredIdentity = serde_json::from_str(&text)
                .with_context(|| format!("身份文件损坏: {}", path.display()))?;
            let seed = hex::decode(stored.seed.trim()).context("身份文件里的 seed 不是合法 hex")?;
            if seed.len() != 32 {
                bail!("身份文件里的 seed 长度错误");
            }
            let mut bytes = [0u8; 32];
            bytes.copy_from_slice(&seed);
            let signing = SigningKey::from_bytes(&bytes);
            let nick = if stored.nick.trim().is_empty() {
                None
            } else {
                Some(stored.nick)
            };
            return Ok((
                Self {
                    signing,
                    created_at: if stored.created_at == 0 {
                        now_ms()
                    } else {
                        stored.created_at
                    },
                },
                nick,
            ));
        }

        let identity = Self::generate();
        let nick = None;
        identity.save(path, "")?;
        Ok((identity, nick))
    }

    /// 保存身份与当前昵称。
    pub fn save(&self, path: &Path, nick: &str) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("创建目录失败: {}", parent.display()))?;
        }
        let stored = StoredIdentity {
            version: 1,
            seed: hex::encode(self.signing.to_bytes()),
            nick: nick.to_string(),
            created_at: self.created_at,
        };
        let text = serde_json::to_string_pretty(&stored)?;
        std::fs::write(path, text)
            .with_context(|| format!("写入身份文件失败: {}", path.display()))?;
        restrict_permissions(path);
        Ok(())
    }

    pub fn pubkey_hex(&self) -> String {
        hex::encode(self.verifying_key().to_bytes())
    }

    pub fn verifying_key(&self) -> VerifyingKey {
        self.signing.verifying_key()
    }

    pub fn peer_id(&self) -> String {
        peer_id(&self.pubkey_hex())
    }

    pub fn short(&self) -> String {
        short_id(&self.pubkey_hex())
    }

    pub fn sign_b64(&self, data: &[u8]) -> String {
        crate::crypto::b64e(&self.signing.sign(data).to_bytes())
    }
}

/// 用签名密钥签一段数据，返回 base64。
pub fn sign_b64(signing: &SigningKey, data: &[u8]) -> String {
    crate::crypto::b64e(&signing.sign(data).to_bytes())
}

/// 用公钥验证一段数据的签名。
pub fn verify_b64(pubkey_hex: &str, data: &[u8], signature_b64: &str) -> Result<()> {
    let key_bytes = hex::decode(pubkey_hex.trim()).context("公钥不是合法 hex")?;
    if key_bytes.len() != 32 {
        bail!("公钥长度错误");
    }
    let mut key_arr = [0u8; 32];
    key_arr.copy_from_slice(&key_bytes);
    let verifying = VerifyingKey::from_bytes(&key_arr).map_err(|e| anyhow!("公钥非法: {e}"))?;
    let sig_bytes = crate::crypto::b64d(signature_b64).context("签名不是合法 base64")?;
    let sig_arr: [u8; 64] = sig_bytes
        .as_slice()
        .try_into()
        .map_err(|_| anyhow!("签名长度错误"))?;
    let signature = Signature::from_bytes(&sig_arr);
    verifying
        .verify_strict(data, &signature)
        .map_err(|_| anyhow!("签名验证失败"))
}

#[cfg(unix)]
fn restrict_permissions(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
fn restrict_permissions(_path: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_and_verify() {
        let id = Identity::generate();
        let sig = id.sign_b64(b"data");
        verify_b64(&id.pubkey_hex(), b"data", &sig).unwrap();
        assert!(verify_b64(&id.pubkey_hex(), b"other", &sig).is_err());
    }

    #[test]
    fn persists_identity() {
        let dir = std::env::temp_dir().join(format!("asa-ident-{}", std::process::id()));
        let path = dir.join("identity.json");
        let _ = std::fs::remove_dir_all(&dir);
        let (id, _) = Identity::load_or_create(&path).unwrap();
        id.save(&path, "alice").unwrap();
        let (again, nick) = Identity::load_or_create(&path).unwrap();
        assert_eq!(id.pubkey_hex(), again.pubkey_hex());
        assert_eq!(nick.as_deref(), Some("alice"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn peer_id_is_stable_across_instances() {
        let dir = std::env::temp_dir().join(format!("asa-ident2-{}", std::process::id()));
        let path = dir.join("identity.json");
        let _ = std::fs::remove_dir_all(&dir);
        let (id, _) = Identity::load_or_create(&path).unwrap();
        let pid = id.peer_id();
        let (again, _) = Identity::load_or_create(&path).unwrap();
        assert_eq!(pid, again.peer_id());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

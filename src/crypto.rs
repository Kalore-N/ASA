//! 群密钥加密（ChaCha20-Poly1305）与编码辅助。

use anyhow::{anyhow, bail, Result};
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine as _;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Nonce};
use sha2::{Digest, Sha256};

/// 32 字节群密钥。持有它即可解密群消息，也是「成员资格」的凭据。
pub type RoomKey = [u8; 32];

pub const AUTH_AAD: &[u8] = b"asa1|auth";

/// 用群密钥加密，返回密文与随机 nonce。
pub fn seal(key: &RoomKey, aad: &[u8], plaintext: &[u8]) -> Result<(Vec<u8>, [u8; 12])> {
    let cipher =
        ChaCha20Poly1305::new_from_slice(key).map_err(|e| anyhow!("初始化加密器失败: {e}"))?;
    let nonce = random_nonce();
    let ciphertext = cipher
        .encrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: plaintext,
                aad,
            },
        )
        .map_err(|_| anyhow!("加密失败"))?;
    Ok((ciphertext, nonce))
}

/// 解密；AAD 必须与加密时一致，否则失败。
pub fn open(key: &RoomKey, aad: &[u8], nonce: &[u8; 12], ciphertext: &[u8]) -> Result<Vec<u8>> {
    let cipher =
        ChaCha20Poly1305::new_from_slice(key).map_err(|e| anyhow!("初始化解密器失败: {e}"))?;
    cipher
        .decrypt(
            Nonce::from_slice(nonce),
            Payload {
                msg: ciphertext,
                aad,
            },
        )
        .map_err(|_| anyhow!("解密失败：密钥不匹配或消息被篡改"))
}

/// 使用调用方提供的 nonce 加密（用于实时语音的计数器 nonce）。
pub fn seal_nonce(
    key: &RoomKey,
    aad: &[u8],
    nonce: &[u8; 12],
    plaintext: &[u8],
) -> Result<Vec<u8>> {
    let cipher =
        ChaCha20Poly1305::new_from_slice(key).map_err(|e| anyhow!("初始化加密器失败: {e}"))?;
    cipher
        .encrypt(
            Nonce::from_slice(nonce),
            Payload {
                msg: plaintext,
                aad,
            },
        )
        .map_err(|_| anyhow!("加密失败"))
}

pub fn open_nonce(
    key: &RoomKey,
    aad: &[u8],
    nonce: &[u8; 12],
    ciphertext: &[u8],
) -> Result<Vec<u8>> {
    let cipher =
        ChaCha20Poly1305::new_from_slice(key).map_err(|e| anyhow!("初始化解密器失败: {e}"))?;
    cipher
        .decrypt(
            Nonce::from_slice(nonce),
            Payload {
                msg: ciphertext,
                aad,
            },
        )
        .map_err(|_| anyhow!("解密失败：密钥不匹配或被篡改"))
}

pub fn random_nonce() -> [u8; 12] {
    use rand::RngCore;
    let mut nonce = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce);
    nonce
}

pub fn random_bytes<const N: usize>() -> [u8; N] {
    use rand::RngCore;
    let mut out = [0u8; N];
    rand::thread_rng().fill_bytes(&mut out);
    out
}

pub fn random_key() -> RoomKey {
    random_bytes::<32>()
}

pub fn sha256_hex(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}

pub fn b64e(data: &[u8]) -> String {
    STANDARD.encode(data)
}

pub fn b64d(text: &str) -> Result<Vec<u8>> {
    STANDARD
        .decode(text.trim())
        .map_err(|e| anyhow!("base64 解码失败: {e}"))
}

pub fn key_to_b64url(key: &RoomKey) -> String {
    URL_SAFE_NO_PAD.encode(key)
}

/// 无 padding 的 URL-safe base64，用于邀请码中的地址参数。
pub fn b64url_encode(data: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(data)
}

pub fn b64url_decode(text: &str) -> Result<Vec<u8>> {
    URL_SAFE_NO_PAD
        .decode(text.trim())
        .map_err(|e| anyhow!("base64url 解码失败: {e}"))
}

pub fn key_from_b64url(text: &str) -> Result<RoomKey> {
    let bytes = URL_SAFE_NO_PAD
        .decode(text.trim())
        .map_err(|e| anyhow!("邀请码中的群密钥格式错误: {e}"))?;
    if bytes.len() != 32 {
        bail!("群密钥长度错误：期望 32 字节，实际 {}", bytes.len());
    }
    let mut key = [0u8; 32];
    key.copy_from_slice(&bytes);
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seal_open_roundtrip() {
        let key = [7u8; 32];
        let (ct, nonce) = seal(&key, b"room", b"hello").unwrap();
        assert_eq!(open(&key, b"room", &nonce, &ct).unwrap(), b"hello");
        assert!(open(&key, b"other", &nonce, &ct).is_err());
    }

    #[test]
    fn tampering_is_detected() {
        let key = [1u8; 32];
        let (mut ct, nonce) = seal(&key, b"", b"payload").unwrap();
        ct[0] ^= 0xff;
        assert!(open(&key, b"", &nonce, &ct).is_err());
    }

    #[test]
    fn b64url_key_roundtrip() {
        let key = [9u8; 32];
        let text = key_to_b64url(&key);
        assert_eq!(key_from_b64url(&text).unwrap(), key);
    }
}

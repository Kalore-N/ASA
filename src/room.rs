//! 群（房间）配置与邀请码。
//!
//! 邀请码形如 `asa://<room_id>/<base64url 群密钥>@<ip>:<port>`。
//! 群密钥就是成员资格：拿到它才能解密消息、加入聊天。

use std::net::SocketAddr;
use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::crypto::{key_from_b64url, key_to_b64url, random_key, RoomKey};
use crate::identity::Identity;
use crate::util::{lan_ipv4, now_ms};

#[derive(Clone, Debug)]
pub struct RoomConfig {
    pub id: String,
    pub key: RoomKey,
    pub created_at: u64,
    pub creator: String,
    /// 已知的群入口地址（局域网、公网或 DDNS）。
    pub entries: Vec<String>,
    /// 中继服务器地址（跨网时使用）。
    pub relays: Vec<String>,
    /// EasyTier 公共节点地址（邀请码里的 `&et=`）。
    pub et_nodes: Vec<String>,
}

#[derive(Serialize, Deserialize)]
struct StoredRoom {
    #[serde(default = "default_version")]
    version: u8,
    id: String,
    key: String,
    created_at: u64,
    creator: String,
    #[serde(default)]
    entries: Vec<String>,
    #[serde(default)]
    relays: Vec<String>,
    #[serde(default)]
    et_nodes: Vec<String>,
}

fn default_version() -> u8 {
    1
}

impl RoomConfig {
    /// 由创建者生成新群。群 ID 是群密钥的哈希前缀。
    pub fn create(identity: &Identity) -> Self {
        let key = random_key();
        Self::from_key(key, now_ms(), identity.pubkey_hex())
    }

    pub fn from_key(key: RoomKey, created_at: u64, creator: String) -> Self {
        let digest = crate::crypto::sha256_hex(&key);
        Self {
            id: digest.chars().take(12).collect(),
            key,
            created_at,
            creator,
            entries: Vec::new(),
            relays: Vec::new(),
            et_nodes: Vec::new(),
        }
    }

    pub fn load(path: &Path) -> Result<Option<Self>> {
        if !path.exists() {
            return Ok(None);
        }
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("读取群配置失败: {}", path.display()))?;
        let stored: StoredRoom = serde_json::from_str(&text)
            .with_context(|| format!("群配置损坏: {}", path.display()))?;
        let key = hex::decode(stored.key.trim()).context("群配置里的密钥不是合法 hex")?;
        if key.len() != 32 {
            bail!("群配置里的密钥长度错误");
        }
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&key);
        Ok(Some(Self {
            id: stored.id,
            key: arr,
            created_at: stored.created_at,
            creator: stored.creator,
            entries: stored.entries,
            relays: stored.relays,
            et_nodes: stored.et_nodes,
        }))
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("创建目录失败: {}", parent.display()))?;
        }
        let stored = StoredRoom {
            version: 1,
            id: self.id.clone(),
            key: hex::encode(self.key),
            created_at: self.created_at,
            creator: self.creator.clone(),
            entries: self.entries.clone(),
            relays: self.relays.clone(),
            et_nodes: self.et_nodes.clone(),
        };
        std::fs::write(path, serde_json::to_string_pretty(&stored)?)
            .with_context(|| format!("写入群配置失败: {}", path.display()))?;
        restrict_permissions(path);
        Ok(())
    }

    /// 生成邀请码；`addr` 为空时尝试用本机局域网地址。
    ///
    /// `?t=` 是群的创建时间（毫秒），`?p=` 是创建者公钥，
    /// 新成员靠它们还原「谁最早在场」的顺序。
    /// `&e=` 重复出现，携带额外入口地址（base64url）。
    pub fn invite(&self, addr: Option<SocketAddr>) -> String {
        let target = addr
            .map(|a| a.to_string())
            .unwrap_or_else(|| format!("{}:7777", lan_ipv4()));
        let mut text = format!(
            "asa://{}/{}@{}?t={}&p={}",
            self.id,
            key_to_b64url(&self.key),
            target,
            self.created_at,
            self.creator
        );
        let mut seen = std::collections::HashSet::new();
        seen.insert(target);
        for entry in &self.entries {
            let entry = entry.trim();
            if entry.is_empty() || !seen.insert(entry.to_string()) {
                continue;
            }
            text.push_str("&e=");
            text.push_str(&crate::crypto::b64url_encode(entry.as_bytes()));
        }
        add_relay_params(&mut text, &self.relays, &mut seen);
        add_et_params(&mut text, &self.et_nodes);
        text
    }

    /// 生成不含易变主地址的长期邀请码，只携带已保存的入口。
    pub fn invite_long(&self) -> String {
        let mut text = format!(
            "asa://{}/{}?t={}&p={}",
            self.id,
            key_to_b64url(&self.key),
            self.created_at,
            self.creator
        );
        let mut seen = std::collections::HashSet::new();
        for entry in &self.entries {
            let entry = entry.trim();
            if entry.is_empty() || !seen.insert(entry.to_string()) {
                continue;
            }
            text.push_str("&e=");
            text.push_str(&crate::crypto::b64url_encode(entry.as_bytes()));
        }
        add_relay_params(&mut text, &self.relays, &mut seen);
        add_et_params(&mut text, &self.et_nodes);
        text
    }

    /// 解析邀请码，返回配置与全部入口地址（主地址在前）。
    pub fn parse_invite_full(text: &str) -> Result<(Self, Vec<String>)> {
        let trimmed = text.trim();
        let body = trimmed
            .strip_prefix("asa://")
            .or_else(|| trimmed.strip_prefix("asa:"))
            .unwrap_or(trimmed);
        if body.is_empty() {
            bail!("邀请码为空");
        }
        let (main, query) = match body.split_once('?') {
            Some((m, q)) => (m, Some(q)),
            None => (body, None),
        };
        let (path, addr) = match main.rsplit_once('@') {
            Some((p, a)) if !a.trim().is_empty() => (p, Some(a.trim().to_string())),
            Some((p, _)) => (p, None),
            None => (main, None),
        };
        let mut parts = path.split('/');
        let room_id = parts.next().unwrap_or("").trim();
        let key_text = parts.next().unwrap_or("").trim();
        if room_id.is_empty() || key_text.is_empty() {
            bail!("邀请码格式错误，示例：asa://<room_id>/<key>@192.168.1.10:7777");
        }
        let key = key_from_b64url(key_text)?;
        let digest = crate::crypto::sha256_hex(&key);
        let derived_id: String = digest.chars().take(12).collect();
        if derived_id != room_id {
            bail!("邀请码校验失败：群 ID 与密钥不匹配");
        }
        let mut created_at = now_ms();
        let mut creator = String::new();
        let mut entries: Vec<String> = Vec::new();
        if let Some(addr) = &addr {
            push_entry(&mut entries, addr);
        }
        let mut relays: Vec<String> = Vec::new();
        let mut et_nodes: Vec<String> = Vec::new();
        if let Some(query) = query {
            for pair in query.split('&') {
                let Some((k, v)) = pair.split_once('=') else {
                    continue;
                };
                match k {
                    "t" => {
                        if let Ok(t) = v.trim().parse::<u64>() {
                            // 防止未来时间导致排序错乱
                            created_at = t.min(now_ms());
                        }
                    }
                    "p" => {
                        let v = v.trim();
                        if v.len() == 64 && hex::decode(v).is_ok() {
                            creator = v.to_string();
                        }
                    }
                    "e" => {
                        if let Ok(bytes) = crate::crypto::b64url_decode(v) {
                            if let Ok(value) = String::from_utf8(bytes) {
                                push_entry(&mut entries, &value);
                            }
                        }
                    }
                    "r" => {
                        if let Ok(bytes) = crate::crypto::b64url_decode(v) {
                            if let Ok(value) = String::from_utf8(bytes) {
                                push_entry(&mut relays, &value);
                            }
                        }
                    }
                    "et" => {
                        if let Ok(bytes) = crate::crypto::b64url_decode(v) {
                            if let Ok(value) = String::from_utf8(bytes) {
                                if crate::easytier::normalize_node(&value).is_ok() {
                                    push_entry(&mut et_nodes, &value);
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        let config = Self {
            id: room_id.to_string(),
            key,
            created_at,
            creator,
            entries: entries.clone(),
            relays,
            et_nodes,
        };
        Ok((config, entries))
    }

    /// 解析邀请码，返回配置与第一个入口地址（兼容旧调用）。
    pub fn parse_invite(text: &str) -> Result<(Self, Option<String>)> {
        let (config, mut entries) = Self::parse_invite_full(text)?;
        let first = if entries.is_empty() {
            None
        } else {
            Some(entries.remove(0))
        };
        Ok((config, first))
    }

    pub fn short_id(&self) -> String {
        self.id.chars().take(6).collect()
    }
}

fn add_relay_params(
    text: &mut String,
    relays: &[String],
    seen: &mut std::collections::HashSet<String>,
) {
    for relay in relays {
        let relay = relay.trim();
        if relay.is_empty() || !seen.insert(relay.to_string()) {
            continue;
        }
        text.push_str("&r=");
        text.push_str(&crate::crypto::b64url_encode(relay.as_bytes()));
    }
}

fn add_et_params(text: &mut String, nodes: &[String]) {
    for node in nodes {
        let node = node.trim();
        if node.is_empty() {
            continue;
        }
        text.push_str("&et=");
        text.push_str(&crate::crypto::b64url_encode(node.as_bytes()));
    }
}

fn push_entry(entries: &mut Vec<String>, value: &str) {
    let value = value.trim();
    if !value.is_empty() && !entries.iter().any(|entry| entry == value) {
        entries.push(value.to_string());
    }
}

#[cfg(unix)]
fn restrict_permissions(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
fn restrict_permissions(_path: &Path) {}

/// 把邀请码里的地址解析成 `SocketAddr`。
pub fn resolve_addr(text: &str) -> Result<SocketAddr> {
    use std::net::ToSocketAddrs;
    if let Ok(addr) = text.parse::<SocketAddr>() {
        return Ok(addr);
    }
    text.to_socket_addrs()
        .with_context(|| format!("无法解析地址: {text}"))?
        .next()
        .ok_or_else(|| anyhow!("地址没有解析结果: {text}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invite_roundtrip() {
        let ident = Identity::generate();
        let room = RoomConfig::create(&ident);
        let text = room.invite(Some("192.168.1.5:7777".parse().unwrap()));
        let (parsed, addr) = RoomConfig::parse_invite(&text).unwrap();
        assert_eq!(parsed.id, room.id);
        assert_eq!(parsed.key, room.key);
        assert_eq!(addr.as_deref(), Some("192.168.1.5:7777"));
    }

    #[test]
    fn invite_preserves_genesis() {
        let ident = Identity::generate();
        let room = RoomConfig::create(&ident);
        let text = room.invite(Some("127.0.0.1:9".parse().unwrap()));
        let (parsed, _) = RoomConfig::parse_invite(&text).unwrap();
        assert_eq!(parsed.created_at, room.created_at);
        assert_eq!(parsed.creator, room.creator);
    }

    #[test]
    fn invite_without_addr() {
        let ident = Identity::generate();
        let room = RoomConfig::create(&ident);
        let text = format!("asa://{}/{}", room.id, key_to_b64url(&room.key));
        let (parsed, addr) = RoomConfig::parse_invite(&text).unwrap();
        assert_eq!(parsed.key, room.key);
        assert!(addr.is_none());
    }

    #[test]
    fn bad_room_id_is_rejected() {
        let ident = Identity::generate();
        let room = RoomConfig::create(&ident);
        let text = format!(
            "asa://deadbeef0000/{}@127.0.0.1:1",
            key_to_b64url(&room.key)
        );
        assert!(RoomConfig::parse_invite(&text).is_err());
    }

    #[test]
    fn invite_carries_easytier_nodes() {
        let ident = Identity::generate();
        let mut room = RoomConfig::create(&ident);
        room.et_nodes = vec![
            "tcp://node.example:11010".to_string(),
            "wss://et.example:443".to_string(),
        ];
        for text in [
            room.invite(Some("10.144.1.2:7777".parse().unwrap())),
            room.invite_long(),
        ] {
            let (parsed, _) = RoomConfig::parse_invite(&text).unwrap();
            assert_eq!(parsed.et_nodes, room.et_nodes);
            assert!(text.contains("&et="));
        }
    }

    #[test]
    fn invite_carries_extra_entries() {
        let ident = Identity::generate();
        let mut room = RoomConfig::create(&ident);
        room.entries = vec![
            "192.168.1.5:7777".to_string(),
            "relay.example.com:9000".to_string(),
        ];
        let text = room.invite(Some("10.0.0.2:7777".parse().unwrap()));
        let (parsed, addresses) = RoomConfig::parse_invite_full(&text).unwrap();
        assert_eq!(
            addresses,
            vec![
                "10.0.0.2:7777".to_string(),
                "192.168.1.5:7777".to_string(),
                "relay.example.com:9000".to_string()
            ]
        );
        assert_eq!(parsed.entries, addresses);
    }

    #[test]
    fn long_invite_has_no_volatile_address() {
        let ident = Identity::generate();
        let mut room = RoomConfig::create(&ident);
        room.entries = vec!["relay.example.com:9000".to_string()];
        let text = room.invite_long();
        assert!(!text.contains('@'));
        let (parsed, addresses) = RoomConfig::parse_invite_full(&text).unwrap();
        assert_eq!(parsed.id, room.id);
        assert_eq!(addresses, vec!["relay.example.com:9000".to_string()]);
    }

    #[test]
    fn legacy_room_config_without_entries_loads() {
        let ident = Identity::generate();
        let room = RoomConfig::create(&ident);
        let dir = std::env::temp_dir().join(format!("asa-room-legacy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("room.json");
        let json = format!(
            r#"{{"version":1,"id":"{}","key":"{}","created_at":{},"creator":"{}"}}"#,
            room.id,
            hex::encode(room.key),
            room.created_at,
            room.creator
        );
        std::fs::write(&path, json).unwrap();
        let loaded = RoomConfig::load(&path).unwrap().unwrap();
        assert!(loaded.entries.is_empty());
        assert_eq!(loaded.id, room.id);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

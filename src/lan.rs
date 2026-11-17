//! 局域网频道发现与密码加入。
//!
//! 宿主通过 mDNS 广播 `_asa-channel._tcp.local.`，客户端选择频道后连接
//! 宿主的临时加入端口，用 PBKDF2 + HMAC 挑战应答换取邀请码。
//!
//! 加入服务只用于局域网发现；真正的聊天仍然走 ASA 自己的加密协议。

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use hmac::{Hmac, Mac};
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use pbkdf2::pbkdf2_hmac;
use serde::{Deserialize, Serialize};
use sha2::Sha256;

use crate::config::{GroupProfile, JoinPassword};
use crate::crypto;
use crate::paths::RuntimeDirs;
use crate::room::RoomConfig;

pub const CHANNEL_SERVICE: &str = "_asa-channel._tcp.local.";
pub const JOIN_ITERATIONS: u32 = 210_000;

type HmacSha256 = Hmac<Sha256>;

#[derive(Clone, Debug, Serialize)]
pub struct DiscoveredChannel {
    pub room: String,
    pub name: String,
    pub host: String,
    /// ASA 聊天端口。
    pub addr: SocketAddr,
    /// 密码加入端口。
    pub join_addr: SocketAddr,
    pub open: bool,
}

impl JoinPassword {
    pub fn new(password: &str) -> Result<Self> {
        let salt: [u8; 16] = crypto::random_bytes();
        let mut key = [0u8; 32];
        pbkdf2_hmac::<Sha256>(password.as_bytes(), &salt, JOIN_ITERATIONS, &mut key);
        Ok(Self {
            salt: crypto::b64url_encode(&salt),
            iterations: JOIN_ITERATIONS,
            hash: crypto::b64url_encode(&key),
        })
    }

    pub fn derive(&self, password: &str) -> Result<[u8; 32]> {
        let salt = crypto::b64url_decode(&self.salt)?;
        let mut key = [0u8; 32];
        pbkdf2_hmac::<Sha256>(password.as_bytes(), &salt, self.iterations.max(1), &mut key);
        Ok(key)
    }

    pub fn verify(&self, password: &str) -> bool {
        let Ok(key) = self.derive(password) else {
            return false;
        };
        let Ok(hash) = crypto::b64url_decode(&self.hash) else {
            return false;
        };
        if key.len() != hash.len() {
            return false;
        }
        let mut diff = 0u8;
        for (a, b) in key.iter().zip(hash.iter()) {
            diff |= a ^ b;
        }
        diff == 0
    }
}

fn derive_key(password: &str, salt_b64: &str, iterations: u32) -> Result<[u8; 32]> {
    let salt = crypto::b64url_decode(salt_b64)?;
    let mut key = [0u8; 32];
    pbkdf2_hmac::<Sha256>(password.as_bytes(), &salt, iterations.max(1), &mut key);
    Ok(key)
}

fn proof(key: &[u8; 32], nonce_b64: &str) -> Result<String> {
    let nonce = crypto::b64url_decode(nonce_b64)?;
    let mut mac = HmacSha256::new_from_slice(key).context("初始化 HMAC 失败")?;
    mac.update(b"asa-join-auth");
    mac.update(&nonce);
    Ok(crypto::b64url_encode(&mac.finalize().into_bytes()))
}

fn verify_proof(key: &[u8; 32], nonce_b64: &str, proof_b64: &str) -> bool {
    let Ok(nonce) = crypto::b64url_decode(nonce_b64) else {
        return false;
    };
    let Ok(bytes) = crypto::b64url_decode(proof_b64) else {
        return false;
    };
    let Ok(mut mac) = HmacSha256::new_from_slice(key) else {
        return false;
    };
    mac.update(b"asa-join-auth");
    mac.update(&nonce);
    mac.verify_slice(&bytes).is_ok()
}

fn invite_key(key: &[u8; 32]) -> [u8; 32] {
    let mut mac = HmacSha256::new_from_slice(key).expect("32 字节 HMAC key");
    mac.update(b"asa-invite-key");
    let bytes = mac.finalize().into_bytes();
    let mut out = [0u8; 32];
    out.copy_from_slice(&bytes[..32]);
    out
}

fn encrypt_invite(key: &[u8; 32], invite: &str) -> Result<(String, String)> {
    let key = invite_key(key);
    let (ciphertext, nonce) = crypto::seal(&key, b"asa1|join-invite", invite.as_bytes())?;
    Ok((
        crypto::b64url_encode(&nonce),
        crypto::b64url_encode(&ciphertext),
    ))
}

fn decrypt_invite(key: &[u8; 32], nonce_b64: &str, ciphertext_b64: &str) -> Result<String> {
    let nonce = crypto::b64url_decode(nonce_b64)?;
    if nonce.len() != 12 {
        bail!("邀请码 nonce 长度错误");
    }
    let mut nonce_arr = [0u8; 12];
    nonce_arr.copy_from_slice(&nonce);
    let ciphertext = crypto::b64url_decode(ciphertext_b64)?;
    let key = invite_key(key);
    let plain = crypto::open(&key, b"asa1|join-invite", &nonce_arr, &ciphertext)?;
    String::from_utf8(plain).context("邀请码不是 UTF-8")
}

/// 把客户端保存的加入密码用频道密钥加密。
pub fn seal_saved_password(room_key: &[u8; 32], password: &str) -> Result<String> {
    let (ciphertext, nonce) = crypto::seal(room_key, b"asa1|join-password", password.as_bytes())?;
    Ok(format!(
        "{}:{}",
        crypto::b64url_encode(&nonce),
        crypto::b64url_encode(&ciphertext)
    ))
}

pub fn open_saved_password(room_key: &[u8; 32], text: &str) -> Result<String> {
    let (nonce_text, cipher_text) = text
        .split_once(':')
        .ok_or_else(|| anyhow!("保存的密码格式错误"))?;
    let nonce = crypto::b64url_decode(nonce_text)?;
    if nonce.len() != 12 {
        bail!("保存的密码 nonce 长度错误");
    }
    let mut nonce_arr = [0u8; 12];
    nonce_arr.copy_from_slice(&nonce);
    let ciphertext = crypto::b64url_decode(cipher_text)?;
    let plain = crypto::open(room_key, b"asa1|join-password", &nonce_arr, &ciphertext)?;
    String::from_utf8(plain).context("保存的密码不是 UTF-8")
}

#[derive(Serialize, Deserialize)]
struct Hello {
    v: u8,
    room: String,
}

#[derive(Serialize, Deserialize)]
struct Challenge {
    v: u8,
    open: bool,
    #[serde(default)]
    salt: String,
    #[serde(default)]
    iterations: u32,
    #[serde(default)]
    nonce: String,
}

#[derive(Serialize, Deserialize)]
struct Proof {
    proof: String,
}

#[derive(Serialize, Deserialize)]
struct JoinReply {
    ok: bool,
    #[serde(default)]
    invite: String,
    #[serde(default)]
    nonce: String,
    #[serde(default)]
    error: String,
}

/// 宿主侧的频道目录 + 加入服务。Drop 时停止线程和 mDNS。
pub struct JoinServer {
    shutdown: Arc<AtomicBool>,
    daemon: ServiceDaemon,
    pub join_port: u16,
}

impl JoinServer {
    pub fn start(
        room: &RoomConfig,
        name: &str,
        nick: &str,
        asa_port: u16,
        invite: &str,
        password: Option<JoinPassword>,
    ) -> Result<Self> {
        let listener = TcpListener::bind(("0.0.0.0", 0)).context("启动频道加入服务失败")?;
        listener.set_nonblocking(true)?;
        let join_port = listener.local_addr()?.port();
        let shutdown = Arc::new(AtomicBool::new(false));
        let room_id = room.id.clone();
        let invite = invite.to_string();
        let has_password = password.is_some();
        let server_password = password.clone();
        let flag = shutdown.clone();
        thread::spawn(move || {
            let failures: Arc<Mutex<HashMap<IpAddr, (u32, Instant)>>> =
                Arc::new(Mutex::new(HashMap::new()));
            while !flag.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, addr)) => {
                        let room_id = room_id.clone();
                        let invite = invite.clone();
                        let password = server_password.clone();
                        let failures = failures.clone();
                        thread::spawn(move || {
                            let _ = handle_client(
                                stream,
                                addr,
                                &room_id,
                                &invite,
                                password.as_ref(),
                                &failures,
                            );
                        });
                    }
                    Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(100));
                    }
                    Err(_) => break,
                }
            }
        });

        let daemon = ServiceDaemon::new().context("创建频道目录 mDNS 失败")?;
        let properties = vec![
            ("room".to_string(), room.id.clone()),
            ("name".to_string(), name.to_string()),
            ("host".to_string(), nick.to_string()),
            ("port".to_string(), asa_port.to_string()),
            ("join".to_string(), join_port.to_string()),
            (
                "open".to_string(),
                if has_password { "0" } else { "1" }.to_string(),
            ),
            ("v".to_string(), "1".to_string()),
        ];
        let instance = format!("asa-{}-{}", room.id, join_port);
        let hostname = format!("asa-{}.local.", room.id);
        let info = ServiceInfo::new(
            CHANNEL_SERVICE,
            &instance,
            &hostname,
            "",
            join_port,
            properties.as_slice(),
        )
        .context("构造频道目录失败")?
        .enable_addr_auto();
        daemon.register(info).context("注册频道目录失败")?;
        Ok(Self {
            shutdown,
            daemon,
            join_port,
        })
    }
}

impl Drop for JoinServer {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        let _ = self.daemon.shutdown();
    }
}

fn read_json_line(reader: &mut BufReader<TcpStream>) -> Result<String> {
    let mut line = String::new();
    let n = reader.read_line(&mut line).context("读取加入请求失败")?;
    if n == 0 {
        bail!("连接已关闭");
    }
    if n > 16 * 1024 {
        bail!("加入请求过长");
    }
    Ok(line)
}

fn write_json_line(stream: &mut TcpStream, value: &impl Serialize) -> Result<()> {
    let text = serde_json::to_string(value)?;
    stream.write_all(text.as_bytes())?;
    stream.write_all(b"\n")?;
    stream.flush()?;
    Ok(())
}

fn handle_client(
    mut stream: TcpStream,
    addr: SocketAddr,
    room_id: &str,
    invite: &str,
    password: Option<&JoinPassword>,
    failures: &Arc<Mutex<HashMap<IpAddr, (u32, Instant)>>>,
) -> Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    stream.set_write_timeout(Some(Duration::from_secs(10)))?;
    {
        let mut guard = failures.lock().unwrap();
        if let Some((count, since)) = guard.get(&addr.ip()) {
            if since.elapsed() < Duration::from_secs(60) && *count >= 5 {
                bail!("尝试次数过多");
            }
        }
        guard.retain(|_, (_, since)| since.elapsed() < Duration::from_secs(60));
    }

    let mut reader = BufReader::new(stream.try_clone()?);
    let hello: Hello =
        serde_json::from_str(&read_json_line(&mut reader)?).context("加入请求格式错误")?;
    if hello.v != 1 || hello.room != room_id {
        write_json_line(
            &mut stream,
            &JoinReply {
                ok: false,
                invite: String::new(),
                nonce: String::new(),
                error: "频道不匹配".to_string(),
            },
        )?;
        return Ok(());
    }

    let nonce = crypto::random_bytes::<16>();
    let nonce_b64 = crypto::b64url_encode(&nonce);
    let challenge = match password {
        Some(password) => Challenge {
            v: 1,
            open: false,
            salt: password.salt.clone(),
            iterations: password.iterations,
            nonce: nonce_b64.clone(),
        },
        None => Challenge {
            v: 1,
            open: true,
            salt: String::new(),
            iterations: 0,
            nonce: String::new(),
        },
    };
    write_json_line(&mut stream, &challenge)?;

    if password.is_none() {
        write_json_line(
            &mut stream,
            &JoinReply {
                ok: true,
                invite: invite.to_string(),
                nonce: String::new(),
                error: String::new(),
            },
        )?;
        return Ok(());
    }

    let proof: Proof =
        serde_json::from_str(&read_json_line(&mut reader)?).context("密码校验格式错误")?;
    let Some(password) = password else {
        return Ok(());
    };
    let key = password_key(password)?;
    if !verify_proof(&key, &nonce_b64, &proof.proof) {
        {
            let mut guard = failures.lock().unwrap();
            let entry = guard.entry(addr.ip()).or_insert((0, Instant::now()));
            entry.0 += 1;
            entry.1 = Instant::now();
        }
        let _ = write_json_line(
            &mut stream,
            &JoinReply {
                ok: false,
                invite: String::new(),
                nonce: String::new(),
                error: "密码错误".to_string(),
            },
        );
        thread::sleep(Duration::from_millis(400));
        return Ok(());
    }

    let (nonce_b64, invite_cipher) = encrypt_invite(&key, invite)?;
    write_json_line(
        &mut stream,
        &JoinReply {
            ok: true,
            invite: invite_cipher,
            nonce: nonce_b64,
            error: String::new(),
        },
    )?;
    Ok(())
}

/// 宿主侧从保存的 PBKDF2 哈希取出挑战应答使用的 32 字节密钥。
fn password_key(password: &JoinPassword) -> Result<[u8; 32]> {
    let bytes = crypto::b64url_decode(&password.hash)?;
    if bytes.len() != 32 {
        bail!("频道密码哈希长度错误");
    }
    let mut key = [0u8; 32];
    key.copy_from_slice(&bytes);
    Ok(key)
}

/// 客户端浏览局域网频道目录。
pub struct ChannelBrowser {
    daemon: ServiceDaemon,
}

impl ChannelBrowser {
    pub fn start() -> Result<(Self, Receiver<DiscoveredChannel>)> {
        let daemon = ServiceDaemon::new().context("创建频道浏览失败")?;
        let receiver = daemon.browse(CHANNEL_SERVICE).context("启动频道浏览失败")?;
        let (tx, rx) = std::sync::mpsc::channel();
        thread::spawn(move || {
            while let Ok(event) = receiver.recv() {
                let ServiceEvent::ServiceResolved(info) = event else {
                    continue;
                };
                let room = info.get_property_val_str("room").unwrap_or("").to_string();
                if room.is_empty() {
                    continue;
                }
                let fallback = format!("频道 {}", room.chars().take(8).collect::<String>());
                let name = info
                    .get_property_val_str("name")
                    .filter(|name| !name.is_empty())
                    .unwrap_or(&fallback)
                    .to_string();
                let host = info.get_property_val_str("host").unwrap_or("").to_string();
                let open = info.get_property_val_str("open") == Some("1");
                let Ok(asa_port) = info
                    .get_property_val_str("port")
                    .unwrap_or("")
                    .parse::<u16>()
                else {
                    continue;
                };
                let Ok(join_port) = info
                    .get_property_val_str("join")
                    .unwrap_or("")
                    .parse::<u16>()
                else {
                    continue;
                };
                for scoped in info.get_addresses() {
                    if !scoped.is_ipv4() {
                        continue;
                    }
                    let ip = scoped.to_ip_addr();
                    let channel = DiscoveredChannel {
                        room: room.clone(),
                        name: name.clone(),
                        host: host.clone(),
                        addr: SocketAddr::new(ip, asa_port),
                        join_addr: SocketAddr::new(ip, join_port),
                        open,
                    };
                    if tx.send(channel).is_err() {
                        return;
                    }
                }
            }
        });
        Ok((Self { daemon }, rx))
    }
}

impl Drop for ChannelBrowser {
    fn drop(&mut self) {
        let _ = self.daemon.shutdown();
    }
}

/// 在指定时间内收集局域网频道（用于 TUI/Android 的短轮询）。
pub fn discover(timeout: Duration) -> Result<Vec<DiscoveredChannel>> {
    let (_browser, rx) = ChannelBrowser::start()?;
    let deadline = Instant::now() + timeout;
    let mut channels: Vec<DiscoveredChannel> = Vec::new();
    while Instant::now() < deadline {
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(channel) => {
                if !channels
                    .iter()
                    .any(|item| item.room == channel.room && item.join_addr == channel.join_addr)
                {
                    channels.push(channel);
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    channels.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.room.cmp(&b.room)));
    Ok(channels)
}

/// 连接宿主加入服务，换取邀请码。
pub fn request_invite(channel: &DiscoveredChannel, password: Option<&str>) -> Result<String> {
    let mut stream = TcpStream::connect_timeout(&channel.join_addr, Duration::from_secs(5))
        .with_context(|| format!("无法连接频道加入服务: {}", channel.join_addr))?;
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    stream.set_write_timeout(Some(Duration::from_secs(10)))?;
    let mut reader = BufReader::new(stream.try_clone()?);

    write_json_line(
        &mut stream,
        &Hello {
            v: 1,
            room: channel.room.clone(),
        },
    )?;
    let challenge: Challenge =
        serde_json::from_str(&read_json_line(&mut reader)?).context("频道加入响应格式错误")?;
    if challenge.v != 1 {
        bail!("加入服务版本不兼容");
    }
    if challenge.open {
        let reply: JoinReply =
            serde_json::from_str(&read_json_line(&mut reader)?).context("邀请码响应格式错误")?;
        if !reply.ok {
            bail!("加入失败: {}", reply.error);
        }
        return Ok(reply.invite);
    }

    let password = password.ok_or_else(|| anyhow!("该频道需要加入密码"))?;
    let key = derive_key(password, &challenge.salt, challenge.iterations)?;
    let proof_text = proof(&key, &challenge.nonce)?;
    write_json_line(&mut stream, &Proof { proof: proof_text })?;
    let reply: JoinReply =
        serde_json::from_str(&read_json_line(&mut reader)?).context("邀请码响应格式错误")?;
    if !reply.ok {
        bail!("加入失败: {}", reply.error);
    }
    decrypt_invite(&key, &reply.nonce, &reply.invite)
}

/// 把加入成功的频道保存到本地，并保存加密后的加入密码。
pub fn save_joined_channel(
    dirs: &RuntimeDirs,
    invite: &str,
    password: Option<&str>,
) -> Result<RoomConfig> {
    let (parsed, _entries) = RoomConfig::parse_invite_full(invite)?;
    let invite_et_nodes = parsed.et_nodes.clone();
    let room = match crate::rooms::load(dirs, &parsed.id)? {
        Some(mut existing) => {
            for entry in &parsed.entries {
                if !existing.entries.iter().any(|item| item == entry) {
                    existing.entries.push(entry.clone());
                }
            }
            for node in &invite_et_nodes {
                if !existing.et_nodes.iter().any(|item| item == node) {
                    existing.et_nodes.push(node.clone());
                }
            }
            existing
        }
        None => parsed,
    };
    crate::rooms::save(dirs, &room)?;
    let mut config = crate::config::Config::ensure(dirs)?;
    let mut pool = crate::easytier::normalize_nodes(config.easytier.nodes.iter());
    if crate::easytier::merge_pool(&mut pool, &invite_et_nodes) {
        config.easytier.nodes = pool.clone();
        config.save(&crate::config::config_path(dirs))?;
    }
    let path = crate::config::group_path(dirs, &room.id);
    let mut profile = GroupProfile::load_at(&path).unwrap_or_default();
    if let Some(password) = password.filter(|value| !value.is_empty()) {
        profile.join_password_enc = Some(seal_saved_password(&room.key, password)?);
    }
    if !pool.is_empty() {
        // 邀请码带 ET 节点时强制开启，避免旧设置导致异地连接失败。
        profile.easytier_enabled = Some(true);
    }
    if profile.et_node.is_none() {
        profile.et_node = crate::easytier::selected_node(&pool, None);
    }
    profile.save_at(&path)?;
    crate::rooms::set_active(dirs, &room.id)?;
    Ok(room)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_verify() {
        let password = JoinPassword::new("correct horse").unwrap();
        assert!(password.verify("correct horse"));
        assert!(!password.verify("wrong"));
    }

    #[test]
    fn proof_roundtrip() {
        let key = [7u8; 32];
        let nonce = crypto::b64url_encode(&crypto::random_bytes::<16>());
        let text = proof(&key, &nonce).unwrap();
        assert!(verify_proof(&key, &nonce, &text));
        assert!(!verify_proof(&[8u8; 32], &nonce, &text));
    }

    #[test]
    fn invite_encrypt_roundtrip() {
        let key = [9u8; 32];
        let (nonce, cipher) = encrypt_invite(&key, "asa://room/key@1.2.3.4:7777").unwrap();
        assert_eq!(
            decrypt_invite(&key, &nonce, &cipher).unwrap(),
            "asa://room/key@1.2.3.4:7777"
        );
    }

    #[test]
    fn saved_password_roundtrip() {
        let key = [3u8; 32];
        let text = seal_saved_password(&key, "hunter2").unwrap();
        assert_eq!(open_saved_password(&key, &text).unwrap(), "hunter2");
        assert!(open_saved_password(&[4u8; 32], &text).is_err());
    }
}

#[cfg(test)]
mod join_server_tests {
    use super::*;
    use crate::identity::Identity;

    #[test]
    fn joined_channel_merges_easytier_nodes() {
        let root = std::env::temp_dir().join(format!("asa-join-et-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let dirs = crate::paths::RuntimeDirs::ensure(&root).unwrap();
        let inviter = Identity::generate();
        let room = RoomConfig::create(&inviter);
        crate::rooms::save(&dirs, &room).unwrap();
        let mut with_nodes = room.clone();
        with_nodes.et_nodes = vec!["tcp://225284.xyz:11010".to_string()];
        let invite = with_nodes.invite(Some("127.0.0.1:7777".parse().unwrap()));
        let saved = save_joined_channel(&dirs, &invite, None).unwrap();
        assert_eq!(saved.et_nodes, vec!["tcp://225284.xyz:11010".to_string()]);
        let profile = GroupProfile::load(&dirs, &room.id).unwrap();
        assert_eq!(profile.easytier_enabled, Some(true));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn password_and_open_join_protocol() {
        let identity = Identity::generate();
        let room = RoomConfig::create(&identity);
        let invite = room.invite(Some("127.0.0.1:7777".parse().unwrap()));
        let password = JoinPassword::new("secret").unwrap();
        let server =
            JoinServer::start(&room, "测试频道", "host", 7777, &invite, Some(password)).unwrap();
        let channel = DiscoveredChannel {
            room: room.id.clone(),
            name: "测试频道".to_string(),
            host: "host".to_string(),
            addr: "127.0.0.1:7777".parse().unwrap(),
            join_addr: format!("127.0.0.1:{}", server.join_port).parse().unwrap(),
            open: false,
        };
        let got = request_invite(&channel, Some("secret")).unwrap();
        let (parsed, _) = RoomConfig::parse_invite_full(&got).unwrap();
        assert_eq!(parsed.id, room.id);
        assert!(request_invite(&channel, Some("wrong")).is_err());
        let found = discover(Duration::from_secs(2)).unwrap_or_default();
        assert!(
            found.iter().any(|item| item.room == room.id),
            "mDNS 应发现频道: {found:?}"
        );
        drop(server);

        let open_server =
            JoinServer::start(&room, "开放频道", "host", 7777, &invite, None).unwrap();
        let open_channel = DiscoveredChannel {
            room: room.id.clone(),
            name: "开放频道".to_string(),
            host: "host".to_string(),
            addr: "127.0.0.1:7777".parse().unwrap(),
            join_addr: format!("127.0.0.1:{}", open_server.join_port)
                .parse()
                .unwrap(),
            open: true,
        };
        let got = request_invite(&open_channel, None).unwrap();
        assert!(got.starts_with("asa://"));
    }
}

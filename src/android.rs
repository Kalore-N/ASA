//! Android JNI 桥接层。
//!
//! Kotlin 侧只通过三个 JSON 接口与 Rust 交互：
//! - `start(config)`：创建/加入/打开群并启动节点
//! - `pollEvents()`：取出自上次轮询以来的所有事件
//! - `sendCommand(cmd)`：发送聊天、文件等命令
//!
//! 这样 JNI 表面很小，Android UI 与 Rust 核心保持解耦。

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use serde_json::json;

use crate::config::{Config, GroupProfile};
use crate::identity::Identity;
use crate::node::{Cmd, Node, NodeConfig, NodeEvent, NodeHandle};
use crate::paths::RuntimeDirs;
use crate::proto::ReplyRef;
use crate::room::{self, RoomConfig};
use crate::settings::Settings;
use crate::util;

struct Bridge {
    runtime: tokio::runtime::Runtime,
    handle: NodeHandle,
    status: serde_json::Value,
}

static BRIDGE: Mutex<Option<Bridge>> = Mutex::new(None);

#[derive(Deserialize)]
struct StartConfig {
    data_dir: String,
    #[serde(default = "default_mode")]
    mode: String,
    nick: Option<String>,
    port: Option<u16>,
    invite: Option<String>,
    #[serde(default)]
    connect: Vec<String>,
    #[serde(default)]
    mdns: Option<bool>,
    max_file: Option<u64>,
    max_file_mb: Option<u64>,
    #[serde(default)]
    download_dir: Option<String>,
    #[serde(default)]
    auto_accept: Option<bool>,
    #[serde(default)]
    relays: Vec<String>,
    #[serde(default)]
    room_id: Option<String>,
}

fn default_mode() -> String {
    "chat".to_string()
}

fn ok(value: serde_json::Value) -> String {
    value.to_string()
}

fn fail(err: anyhow::Error) -> String {
    json!({ "ok": false, "error": format!("{err:#}") }).to_string()
}

fn start_inner(config_json: &str) -> String {
    match start_node(config_json) {
        Ok(value) => ok(value),
        Err(err) => fail(err),
    }
}

fn start_node(config_json: &str) -> Result<serde_json::Value> {
    stop_inner();
    let cfg: StartConfig = serde_json::from_str(config_json).context("启动参数不是合法 JSON")?;
    if cfg.data_dir.trim().is_empty() {
        return Err(anyhow!("数据目录不能为空"));
    }
    let root = PathBuf::from(&cfg.data_dir);
    let dirs = RuntimeDirs::ensure(&root)?;
    crate::rooms::migrate(&dirs)?;
    let config = Config::ensure(&dirs)?;
    let (identity, saved_nick) = Identity::load_or_create(&dirs.identity_path())?;

    let settings = Settings::load(&dirs.settings_path()).unwrap_or_default();
    let port = cfg
        .port
        .or(config.network.port)
        .or(settings.port)
        .unwrap_or(7777);
    let _ = Settings { port: Some(port) }.save(&dirs.settings_path());

    let mut invite_addresses: Vec<String> = Vec::new();
    let room = match cfg.mode.as_str() {
        "create" => {
            // Android 的「创建群聊」每次都生成新群，不再复用旧 room.json。
            let room = RoomConfig::create(&identity);
            crate::rooms::save(&dirs, &room)?;
            room
        }
        "join" => {
            let text = cfg
                .invite
                .as_deref()
                .ok_or_else(|| anyhow!("加入频道需要邀请码"))?;
            let (parsed, addresses) = RoomConfig::parse_invite_full(text)?;
            invite_addresses = addresses;
            let room = match crate::rooms::load(&dirs, &parsed.id)? {
                Some(mut existing) => {
                    for entry in &parsed.entries {
                        if !existing.entries.iter().any(|e| e == entry) {
                            existing.entries.push(entry.clone());
                        }
                    }
                    for relay in &parsed.relays {
                        if !existing.relays.iter().any(|r| r == relay) {
                            existing.relays.push(relay.clone());
                        }
                    }
                    crate::rooms::save(&dirs, &existing)?;
                    existing
                }
                None => {
                    crate::rooms::save(&dirs, &parsed)?;
                    parsed
                }
            };
            room
        }
        "chat" => match cfg.room_id.as_deref() {
            Some(id) => {
                crate::rooms::load(&dirs, id)?.ok_or_else(|| anyhow!("找不到本地频道: {id}"))?
            }
            None => {
                crate::rooms::active(&dirs)?.ok_or_else(|| anyhow!("还没有频道，请先创建或加入"))?
            }
        },
        other => return Err(anyhow!("未知模式: {other}")),
    };

    let mut profile = GroupProfile::load(&dirs, &room.id).unwrap_or_default();
    let nick = cfg
        .nick
        .clone()
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty())
        .or_else(|| profile.nick.clone())
        .or_else(|| {
            saved_nick
                .map(|n| n.trim().to_string())
                .filter(|n| !n.is_empty())
        })
        .unwrap_or_else(|| format!("旅行者{}", identity.short()));
    identity.save(&dirs.identity_path(), &nick)?;
    if profile.nick.as_deref() != Some(nick.as_str()) {
        profile.nick = Some(nick.clone());
        let _ = profile.save(&dirs, &room.id);
    }
    let downloads = profile.download_dir(&config, &dirs);
    let max_file = cfg
        .max_file_mb
        .map(|mb| {
            if mb == 0 {
                u64::MAX
            } else {
                mb.saturating_mul(1024 * 1024)
            }
        })
        .or(cfg.max_file)
        .unwrap_or_else(|| profile.max_file(&config));
    let auto_accept = cfg
        .auto_accept
        .unwrap_or_else(|| profile.auto_accept(&config));
    let mdns = cfg.mdns.unwrap_or(config.network.mdns);
    let mut relays = config.network.relays.clone();
    for relay in room.relays.iter().chain(cfg.relays.iter()) {
        if !relays.iter().any(|item| item == relay) {
            relays.push(relay.clone());
        }
    }

    let pubkey = identity.pubkey_hex();
    let self_since = if !room.creator.is_empty() && room.creator == pubkey {
        room.created_at
    } else {
        util::now_ms()
    };
    let bind: SocketAddr = format!("0.0.0.0:{port}").parse().context("监听地址无效")?;
    let advertise = Some(SocketAddr::new(util::lan_ipv4(), port));

    let mut bootstrap = Vec::new();
    for text in &invite_addresses {
        if let Ok(addr) = room::resolve_addr(text) {
            if !bootstrap.contains(&addr) {
                bootstrap.push(addr);
            }
        }
    }
    for item in &cfg.connect {
        for part in item.split(',') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            if let Ok(addr) = room::resolve_addr(part) {
                if !bootstrap.contains(&addr) {
                    bootstrap.push(addr);
                }
            }
        }
    }

    let node_cfg = NodeConfig {
        signing: identity.signing.clone(),
        pubkey: pubkey.clone(),
        nick: Arc::new(RwLock::new(nick.clone())),
        room_id: room.id.clone(),
        room_key: room.key,
        room_created: room.created_at,
        creator: room.creator.clone(),
        self_since,
        bind,
        advertise,
        dirs: dirs.clone(),
        identity_path: dirs.identity_path(),
        downloads: downloads.clone(),
        max_file,
        auto_accept,
        mdns,
        bootstrap,
        sync_history: config.sync.enabled,
        sync_count: config.sync.count,
        relays,
    };

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("asa-android")
        .build()
        .context("创建异步运行时失败")?;
    let handle = runtime.block_on(Node::start(node_cfg))?;
    let groups = crate::rooms::list(&dirs).unwrap_or_default();
    let info = json!({
        "ok": true,
        "room_id": handle.room_id,
        "groups": groups,
        "peer_id": handle.peer_id,
        "pubkey": handle.pubkey,
        "listen": handle.listen.to_string(),
        "advertise": handle.advertise.to_string(),
        "invite": handle.invite,
        "nick": nick,
        "room_name": profile.name,
        "data_dir": root.display().to_string(),
        "downloads": downloads.display().to_string(),
    });
    let status = info.clone();
    *BRIDGE.lock().unwrap() = Some(Bridge {
        runtime,
        handle,
        status,
    });
    Ok(info)
}

fn stop_inner() {
    let bridge = BRIDGE.lock().unwrap().take();
    if let Some(bridge) = bridge {
        let _ = bridge.handle.cmd.send(Cmd::Quit);
        bridge.runtime.shutdown_timeout(Duration::from_secs(2));
    }
}

fn event_json(event: NodeEvent) -> serde_json::Value {
    match event {
        NodeEvent::Chat {
            from,
            nick,
            text,
            ts,
            mine,
            id,
            mentions,
            reply,
            history,
        } => json!({
            "type": "chat",
            "from": from,
            "nick": nick,
            "text": text,
            "ts": ts,
            "mine": mine,
            "id": id,
            "mentions": mentions,
            "reply": reply,
            "history": history,
        }),
        NodeEvent::System { text, ts } => {
            json!({ "type": "system", "text": text, "ts": ts })
        }
        NodeEvent::Members { members, host, me } => {
            let list: Vec<serde_json::Value> = members
                .iter()
                .map(|m| {
                    json!({
                        "pubkey": m.pubkey,
                        "peer_id": m.peer_id,
                        "nick": m.nick,
                        "addr": m.addr,
                        "host": m.host,
                        "me": m.me,
                        "presence": m.presence,
                    })
                })
                .collect();
            json!({
                "type": "members",
                "members": list,
                "host": host,
                "me": me,
            })
        }
        NodeEvent::FileOffer {
            xfer,
            from,
            from_nick,
            name,
            size,
        } => json!({
            "type": "file_offer",
            "xfer": xfer,
            "from": from,
            "from_nick": from_nick,
            "name": name,
            "size": size,
        }),
        NodeEvent::FileProgress {
            xfer,
            name,
            done,
            total,
            outgoing,
        } => json!({
            "type": "file_progress",
            "xfer": xfer,
            "name": name,
            "done": done,
            "total": total,
            "outgoing": outgoing,
        }),
        NodeEvent::FileComplete {
            xfer,
            ok,
            msg,
            outgoing,
            path,
            name,
        } => json!({
            "type": "file_complete",
            "xfer": xfer,
            "ok": ok,
            "msg": msg,
            "outgoing": outgoing,
            "path": path.map(|p| p.display().to_string()),
            "name": name,
        }),
        NodeEvent::RoomName(name) => json!({ "type": "room_name", "name": name }),
        NodeEvent::Info(text) => json!({ "type": "info", "text": text }),
        NodeEvent::Shutdown => json!({ "type": "shutdown" }),
    }
}

fn poll_events_inner() -> String {
    let mut events = Vec::new();
    if let Some(bridge) = BRIDGE.lock().unwrap().as_mut() {
        while let Ok(event) = bridge.handle.events.try_recv() {
            events.push(event_json(event));
        }
    }
    serde_json::to_string(&events).unwrap_or_else(|_| "[]".to_string())
}

#[derive(Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
enum AndroidCommand {
    Chat {
        text: String,
        reply: Option<ReplyRef>,
    },
    SetNick {
        nick: String,
    },
    SetRoomName {
        name: String,
    },
    Connect {
        addr: String,
    },
    SendFile {
        target: String,
        path: String,
    },
    SendFileMany {
        targets: Vec<String>,
        path: String,
    },
    Accept {
        xfer: String,
    },
    Reject {
        xfer: String,
        #[serde(default)]
        reason: Option<String>,
    },
    Quit,
}

fn send_command_inner(json_text: &str) -> String {
    let parsed: AndroidCommand = match serde_json::from_str(json_text) {
        Ok(cmd) => cmd,
        Err(err) => return fail(err.into()),
    };
    let cmd = match parsed {
        AndroidCommand::Chat { text, reply } => Cmd::Chat { text, reply },
        AndroidCommand::SetNick { nick } => Cmd::SetNick(nick),
        AndroidCommand::SetRoomName { name } => Cmd::SetRoomName(name),
        AndroidCommand::Connect { addr } => Cmd::Connect(addr),
        AndroidCommand::SendFile { target, path } => Cmd::SendFile {
            target,
            path: PathBuf::from(path),
        },
        AndroidCommand::SendFileMany { targets, path } => Cmd::SendFileMany {
            targets,
            path: PathBuf::from(path),
        },
        AndroidCommand::Accept { xfer } => Cmd::Accept(xfer),
        AndroidCommand::Reject { xfer, reason } => {
            let _ = reason;
            Cmd::Reject(xfer)
        }
        AndroidCommand::Quit => Cmd::Quit,
    };
    let bridge = BRIDGE.lock().unwrap();
    let Some(bridge) = bridge.as_ref() else {
        return fail(anyhow!("节点尚未启动"));
    };
    match bridge.handle.cmd.send(cmd) {
        Ok(()) => json!({ "ok": true }).to_string(),
        Err(_) => fail(anyhow!("节点已停止")),
    }
}

// ---------------------------------------------------------------- JNI

fn read_string(env: &mut jni::JNIEnv<'_>, value: &jni::objects::JString<'_>) -> Result<String> {
    let text = env.get_string(value).context("读取 Java 字符串失败")?;
    Ok(text.into())
}

fn to_jstring(env: &mut jni::JNIEnv<'_>, text: String) -> jni::sys::jstring {
    match env.new_string(text) {
        Ok(value) => value.into_raw(),
        Err(_) => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "system" fn Java_com_asa_chat_NativeBridge_start<'local>(
    mut env: jni::JNIEnv<'local>,
    _this: jni::objects::JObject<'local>,
    config: jni::objects::JString<'local>,
) -> jni::sys::jstring {
    let result = match read_string(&mut env, &config) {
        Ok(text) => start_inner(&text),
        Err(err) => fail(err),
    };
    to_jstring(&mut env, result)
}

#[no_mangle]
pub extern "system" fn Java_com_asa_chat_NativeBridge_pollEvents<'local>(
    mut env: jni::JNIEnv<'local>,
    _this: jni::objects::JObject<'local>,
) -> jni::sys::jstring {
    to_jstring(&mut env, poll_events_inner())
}

#[no_mangle]
pub extern "system" fn Java_com_asa_chat_NativeBridge_sendCommand<'local>(
    mut env: jni::JNIEnv<'local>,
    _this: jni::objects::JObject<'local>,
    command: jni::objects::JString<'local>,
) -> jni::sys::jstring {
    let result = match read_string(&mut env, &command) {
        Ok(text) => send_command_inner(&text),
        Err(err) => fail(err),
    };
    to_jstring(&mut env, result)
}

fn groups_json(data_dir: &str) -> String {
    let root = PathBuf::from(data_dir);
    match RuntimeDirs::ensure(&root)
        .and_then(|dirs| crate::rooms::migrate(&dirs))
        .and_then(|_| RuntimeDirs::ensure(&root))
        .and_then(|dirs| crate::rooms::list(&dirs))
    {
        Ok(groups) => json!({ "ok": true, "groups": groups }).to_string(),
        Err(err) => fail(err),
    }
}

#[no_mangle]
pub extern "system" fn Java_com_asa_chat_NativeBridge_groups<'local>(
    mut env: jni::JNIEnv<'local>,
    _this: jni::objects::JObject<'local>,
    data_dir: jni::objects::JString<'local>,
) -> jni::sys::jstring {
    let result = match read_string(&mut env, &data_dir) {
        Ok(dir) => groups_json(&dir),
        Err(err) => fail(err),
    };
    to_jstring(&mut env, result)
}

#[no_mangle]
pub extern "system" fn Java_com_asa_chat_NativeBridge_renameGroup<'local>(
    mut env: jni::JNIEnv<'local>,
    _this: jni::objects::JObject<'local>,
    data_dir: jni::objects::JString<'local>,
    room_id: jni::objects::JString<'local>,
    name: jni::objects::JString<'local>,
) -> jni::sys::jstring {
    let result = (|| -> Result<serde_json::Value> {
        let root = PathBuf::from(read_string(&mut env, &data_dir)?);
        let room_id = read_string(&mut env, &room_id)?;
        let name = read_string(&mut env, &name)?;
        let name: String = crate::util::sanitize_text(&name).chars().take(40).collect();
        if name.is_empty() {
            return Err(anyhow!("频道名称不能为空"));
        }
        let dirs = RuntimeDirs::ensure(&root)?;
        let mut profile = GroupProfile::load(&dirs, &room_id)?;
        profile.name = Some(name);
        profile.save(&dirs, &room_id)?;
        Ok(json!({ "ok": true }))
    })();
    to_jstring(
        &mut env,
        match result {
            Ok(value) => value.to_string(),
            Err(err) => fail(err),
        },
    )
}

#[no_mangle]
pub extern "system" fn Java_com_asa_chat_NativeBridge_isRunning<'local>(
    _env: jni::JNIEnv<'local>,
    _this: jni::objects::JObject<'local>,
) -> jni::sys::jboolean {
    if BRIDGE.lock().unwrap().is_some() {
        1
    } else {
        0
    }
}

#[no_mangle]
pub extern "system" fn Java_com_asa_chat_NativeBridge_status<'local>(
    mut env: jni::JNIEnv<'local>,
    _this: jni::objects::JObject<'local>,
) -> jni::sys::jstring {
    let text = BRIDGE
        .lock()
        .unwrap()
        .as_ref()
        .map(|bridge| bridge.status.to_string())
        .unwrap_or_else(|| json!({ "running": false }).to_string());
    to_jstring(&mut env, text)
}

#[no_mangle]
pub extern "system" fn Java_com_asa_chat_NativeBridge_stop<'local>(
    _env: jni::JNIEnv<'local>,
    _this: jni::objects::JObject<'local>,
) {
    stop_inner();
}

#[no_mangle]
pub extern "system" fn Java_com_asa_chat_NativeBridge_version<'local>(
    mut env: jni::JNIEnv<'local>,
    _this: jni::objects::JObject<'local>,
) -> jni::sys::jstring {
    to_jstring(
        &mut env,
        json!({ "version": env!("CARGO_PKG_VERSION") }).to_string(),
    )
}

/// 供 Rust 单元测试使用：不经过 JNI 也能跑通启动/停止。
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_requires_data_dir() {
        let result = start_inner(r#"{"data_dir":""}"#);
        assert!(result.contains("\"ok\":false"));
    }

    #[test]
    fn send_without_node_fails() {
        let result = send_command_inner(r#"{"cmd":"chat","text":"hi"}"#);
        assert!(result.contains("\"ok\":false"));
    }
}

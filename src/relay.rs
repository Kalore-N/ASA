//! 轻量中继：帮助没有公网 IP 的节点跨网连接。
//!
//! 中继只负责配对与转发 TCP 字节流，双方仍然运行原有的 ASA 握手与
//! 端到端加密，中继看不到聊天内容。
//!
//! 控制连接协议（长度前缀 JSON）：
//! - 客户端 `Hello` 注册 room + pubkey
//! - 客户端 `Lookup` 获取同群在线节点
//! - 客户端 `Connect` 请求连接某个节点
//! - 目标收到 `Incoming` 后建立数据连接，发起方收到 `Paired`
//! 数据连接首帧为 `Data { conn, role }`，之后中继双向复制字节。

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, Mutex};

use crate::crypto;
use crate::net::{self, HandshakeCtx, NetEvent};
use crate::util::now_ms;

const MAX_MSG: usize = 64 * 1024;
const PENDING_TTL: Duration = Duration::from_secs(30);

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "k", rename_all = "snake_case")]
pub enum RelayMsg {
    Hello {
        room: String,
        pubkey: String,
        addr: String,
    },
    Welcome {
        relay: String,
    },
    Lookup {
        room: String,
    },
    Peers {
        peers: Vec<RelayPeer>,
    },
    Connect {
        room: String,
        target: String,
    },
    Paired {
        conn: String,
    },
    Incoming {
        conn: String,
        from: String,
    },
    Data {
        conn: String,
        role: String,
    },
    Error {
        msg: String,
    },
    Ping,
    Pong,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RelayPeer {
    pub pubkey: String,
    pub addr: String,
}

#[derive(Debug)]
pub enum RelayCmd {
    Lookup,
    Connect(String),
    Stop,
}

async fn write_msg<W: AsyncWrite + Unpin>(writer: &mut W, msg: &RelayMsg) -> Result<()> {
    let body = serde_json::to_vec(msg)?;
    if body.len() > MAX_MSG {
        bail!("中继消息过大");
    }
    writer.write_all(&(body.len() as u32).to_be_bytes()).await?;
    writer.write_all(&body).await?;
    writer.flush().await?;
    Ok(())
}

async fn read_msg<R: AsyncRead + Unpin>(reader: &mut R) -> Result<Option<RelayMsg>> {
    let mut len_buf = [0u8; 4];
    if let Err(err) = reader.read_exact(&mut len_buf).await {
        if err.kind() == std::io::ErrorKind::UnexpectedEof {
            return Ok(None);
        }
        return Err(err.into());
    }
    let len = u32::from_be_bytes(len_buf) as usize;
    if len == 0 || len > MAX_MSG {
        bail!("非法中继消息长度: {len}");
    }
    let mut body = vec![0u8; len];
    reader.read_exact(&mut body).await?;
    let msg = serde_json::from_slice(&body).context("中继消息解析失败")?;
    Ok(Some(msg))
}

// ---------------------------------------------------------------- 服务端

struct ClientHandle {
    tx: mpsc::Sender<RelayMsg>,
    addr: String,
}

#[derive(Default)]
struct Pending {
    requester: Option<TcpStream>,
    target: Option<TcpStream>,
    created: Option<Instant>,
}

struct RelayState {
    rooms: Mutex<HashMap<String, HashMap<String, ClientHandle>>>,
    pending: Mutex<HashMap<String, Pending>>,
    known: Mutex<HashMap<String, u64>>,
    state_path: Option<PathBuf>,
}

impl RelayState {
    fn new(state_path: Option<PathBuf>) -> Self {
        Self {
            rooms: Mutex::new(HashMap::new()),
            pending: Mutex::new(HashMap::new()),
            known: Mutex::new(HashMap::new()),
            state_path,
        }
    }

    async fn load(&self) {
        let Some(path) = &self.state_path else {
            return;
        };
        let Ok(text) = std::fs::read_to_string(path) else {
            return;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
            return;
        };
        if let Some(rooms) = value.get("rooms").and_then(|v| v.as_object()) {
            let mut known = self.known.lock().await;
            for (room, ts) in rooms {
                if let Some(ts) = ts.as_u64() {
                    known.insert(room.clone(), ts);
                }
            }
        }
    }

    async fn save_known(&self) {
        let Some(path) = &self.state_path else {
            return;
        };
        let known = self.known.lock().await.clone();
        let value = serde_json::json!({ "rooms": known });
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(
            path,
            serde_json::to_string_pretty(&value).unwrap_or_default(),
        );
    }

    async fn register(&self, room: &str, pubkey: &str, client: ClientHandle) {
        self.rooms
            .lock()
            .await
            .entry(room.to_string())
            .or_default()
            .insert(pubkey.to_string(), client);
        self.known.lock().await.insert(room.to_string(), now_ms());
        self.save_known().await;
    }

    async fn unregister(&self, room: &str, pubkey: &str) {
        let mut rooms = self.rooms.lock().await;
        if let Some(clients) = rooms.get_mut(room) {
            clients.remove(pubkey);
            if clients.is_empty() {
                rooms.remove(room);
            }
        }
    }

    async fn peers(&self, room: &str, exclude: &str) -> Vec<RelayPeer> {
        self.rooms
            .lock()
            .await
            .get(room)
            .map(|clients| {
                clients
                    .iter()
                    .filter(|(pubkey, _)| pubkey.as_str() != exclude)
                    .map(|(pubkey, client)| RelayPeer {
                        pubkey: pubkey.clone(),
                        addr: client.addr.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    async fn connect(&self, room: &str, from: &str, target: &str, tx: &mpsc::Sender<RelayMsg>) {
        let target_client = {
            let rooms = self.rooms.lock().await;
            rooms
                .get(room)
                .and_then(|clients| clients.get(target))
                .map(|client| client.tx.clone())
        };
        let Some(target_tx) = target_client else {
            let _ = tx
                .send(RelayMsg::Error {
                    msg: "目标不在线".to_string(),
                })
                .await;
            return;
        };
        let conn = hex::encode(crypto::random_bytes::<16>());
        let mut pending = self.pending.lock().await;
        pending.insert(
            conn.clone(),
            Pending {
                requester: None,
                target: None,
                created: Some(Instant::now()),
            },
        );
        drop(pending);

        let incoming = RelayMsg::Incoming {
            conn: conn.clone(),
            from: from.to_string(),
        };
        if target_tx.send(incoming).await.is_err() {
            self.pending.lock().await.remove(&conn);
            let _ = tx
                .send(RelayMsg::Error {
                    msg: "目标连接已断开".to_string(),
                })
                .await;
            return;
        }
        let _ = tx.send(RelayMsg::Paired { conn }).await;
    }
}

pub async fn run_relay(bind: SocketAddr, state_path: Option<PathBuf>) -> Result<()> {
    let listener = TcpListener::bind(bind)
        .await
        .with_context(|| format!("中继监听失败: {bind}"))?;
    println!("ASA relay 已监听 {bind}");
    if let Some(path) = &state_path {
        println!("状态文件: {}", path.display());
    }

    let state = Arc::new(RelayState::new(state_path));
    state.load().await;
    {
        let state = state.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(15)).await;
                let mut pending = state.pending.lock().await;
                pending.retain(|_, item| {
                    item.created
                        .map(|created| created.elapsed() < PENDING_TTL)
                        .unwrap_or(false)
                });
            }
        });
    }

    loop {
        let (stream, addr) = listener.accept().await?;
        let state = state.clone();
        tokio::spawn(async move {
            if let Err(err) = handle_incoming(stream, addr, state).await {
                // 客户端断开/格式错误是常态，不打印堆栈。
                let _ = err;
            }
        });
    }
}

async fn handle_incoming(
    stream: TcpStream,
    addr: SocketAddr,
    state: Arc<RelayState>,
) -> Result<()> {
    stream.set_nodelay(true).ok();
    let mut stream = stream;
    let Some(first) = read_msg(&mut stream).await? else {
        return Ok(());
    };
    match first {
        RelayMsg::Hello {
            room,
            pubkey,
            addr: listen,
        } => {
            if room.trim().is_empty() || pubkey.len() != 64 {
                bail!("非法注册信息");
            }
            handle_control(stream, room, pubkey, listen, state).await
        }
        RelayMsg::Data { conn, role } => handle_data(stream, conn, role, state).await,
        _ => {
            let _ = addr;
            Ok(())
        }
    }
}

async fn handle_control(
    stream: TcpStream,
    room: String,
    pubkey: String,
    addr: String,
    state: Arc<RelayState>,
) -> Result<()> {
    let (mut reader, mut writer) = stream.into_split();
    let (tx, mut rx) = mpsc::channel::<RelayMsg>(64);
    let writer_task = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if write_msg(&mut writer, &msg).await.is_err() {
                break;
            }
        }
    });

    state
        .register(
            &room,
            &pubkey,
            ClientHandle {
                tx: tx.clone(),
                addr,
            },
        )
        .await;
    let _ = tx
        .send(RelayMsg::Welcome {
            relay: String::new(),
        })
        .await;

    let result = async {
        loop {
            match read_msg(&mut reader).await? {
                Some(RelayMsg::Lookup { room: query }) => {
                    let peers = state.peers(&query, &pubkey).await;
                    if tx.send(RelayMsg::Peers { peers }).await.is_err() {
                        break;
                    }
                }
                Some(RelayMsg::Connect {
                    room: query,
                    target,
                }) => {
                    state.connect(&query, &pubkey, &target, &tx).await;
                }
                Some(RelayMsg::Ping) => {
                    let _ = tx.send(RelayMsg::Pong).await;
                }
                Some(_) => {}
                None => break,
            }
        }
        Ok::<(), anyhow::Error>(())
    }
    .await;

    state.unregister(&room, &pubkey).await;
    drop(tx);
    writer_task.abort();
    result
}

async fn handle_data(
    stream: TcpStream,
    conn: String,
    role: String,
    state: Arc<RelayState>,
) -> Result<()> {
    if role != "requester" && role != "target" {
        bail!("非法数据连接角色");
    }
    let pair = {
        let mut pending = state.pending.lock().await;
        let Some(item) = pending.get_mut(&conn) else {
            bail!("未知中继连接: {conn}");
        };
        if role == "requester" {
            item.requester = Some(stream);
        } else {
            item.target = Some(stream);
        }
        if item.requester.is_some() && item.target.is_some() {
            Some((item.requester.take().unwrap(), item.target.take().unwrap()))
        } else {
            None
        }
    };
    if let Some((mut a, mut b)) = pair {
        state.pending.lock().await.remove(&conn);
        let _ = tokio::io::copy_bidirectional(&mut a, &mut b).await;
    }
    Ok(())
}

// ---------------------------------------------------------------- 客户端

pub fn spawn_client(
    relay: SocketAddr,
    room: String,
    pubkey: String,
    listen: String,
    ctx: Arc<HandshakeCtx>,
    events: mpsc::Sender<NetEvent>,
) -> mpsc::Sender<RelayCmd> {
    let (tx, mut rx) = mpsc::channel::<RelayCmd>(32);
    tokio::spawn(async move {
        loop {
            let keep_going = run_control(
                relay,
                &room,
                &pubkey,
                &listen,
                ctx.clone(),
                events.clone(),
                &mut rx,
            )
            .await;
            if !keep_going {
                break;
            }
            let _ = events
                .send(NetEvent::Notice(format!(
                    "中继连接断开，5 秒后重试: {relay}"
                )))
                .await;
            match tokio::time::timeout(Duration::from_secs(5), rx.recv()).await {
                Ok(Some(RelayCmd::Stop)) | Ok(None) => break,
                _ => {}
            }
        }
    });
    tx
}

/// 返回 true 表示应继续重连，false 表示收到 Stop。
async fn run_control(
    relay: SocketAddr,
    room: &str,
    pubkey: &str,
    listen: &str,
    ctx: Arc<HandshakeCtx>,
    events: mpsc::Sender<NetEvent>,
    commands: &mut mpsc::Receiver<RelayCmd>,
) -> bool {
    let connect = tokio::time::timeout(Duration::from_secs(8), TcpStream::connect(relay)).await;
    let Ok(Ok(stream)) = connect else {
        return true;
    };
    stream.set_nodelay(true).ok();
    let (mut reader, mut writer) = stream.into_split();
    let hello = RelayMsg::Hello {
        room: room.to_string(),
        pubkey: pubkey.to_string(),
        addr: listen.to_string(),
    };
    if write_msg(&mut writer, &hello).await.is_err() {
        return true;
    }

    let (out_tx, mut out_rx) = mpsc::channel::<RelayMsg>(64);
    let writer_task = tokio::spawn(async move {
        while let Some(msg) = out_rx.recv().await {
            if write_msg(&mut writer, &msg).await.is_err() {
                break;
            }
        }
    });
    let _ = out_tx
        .send(RelayMsg::Lookup {
            room: room.to_string(),
        })
        .await;

    let mut tick = tokio::time::interval(Duration::from_secs(10));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            command = commands.recv() => {
                match command {
                    Some(RelayCmd::Lookup) => {
                        let _ = out_tx.send(RelayMsg::Lookup { room: room.to_string() }).await;
                    }
                    Some(RelayCmd::Connect(target)) => {
                        let _ = out_tx.send(RelayMsg::Connect { room: room.to_string(), target }).await;
                    }
                    Some(RelayCmd::Stop) | None => {
                        drop(out_tx);
                        writer_task.abort();
                        return false;
                    }
                }
            }
            _ = tick.tick() => {
                let _ = out_tx.send(RelayMsg::Lookup { room: room.to_string() }).await;
            }
            message = read_msg(&mut reader) => {
                match message {
                    Ok(Some(RelayMsg::Welcome { .. })) => {
                        let _ = out_tx.send(RelayMsg::Lookup { room: room.to_string() }).await;
                    }
                    Ok(Some(RelayMsg::Peers { peers })) => {
                        let _ = events.send(NetEvent::RelayPeers(peers)).await;
                    }
                    Ok(Some(RelayMsg::Incoming { conn, .. })) => {
                        spawn_data(relay, conn, "target", true, ctx.clone(), events.clone());
                    }
                    Ok(Some(RelayMsg::Paired { conn })) => {
                        spawn_data(relay, conn, "requester", false, ctx.clone(), events.clone());
                    }
                    Ok(Some(RelayMsg::Error { msg })) => {
                        let _ = events.send(NetEvent::Notice(format!("中继: {msg}"))).await;
                    }
                    Ok(Some(_)) => {}
                    Ok(None) | Err(_) => break,
                }
            }
        }
    }

    drop(out_tx);
    writer_task.abort();
    true
}

fn spawn_data(
    relay: SocketAddr,
    conn: String,
    role: &'static str,
    inbound: bool,
    ctx: Arc<HandshakeCtx>,
    events: mpsc::Sender<NetEvent>,
) {
    tokio::spawn(async move {
        let stream = tokio::time::timeout(Duration::from_secs(8), TcpStream::connect(relay)).await;
        let Ok(Ok(mut stream)) = stream else {
            let _ = events
                .send(NetEvent::Notice("中继数据连接失败".to_string()))
                .await;
            return;
        };
        stream.set_nodelay(true).ok();
        let header = RelayMsg::Data {
            conn,
            role: role.to_string(),
        };
        if write_msg(&mut stream, &header).await.is_err() {
            return;
        }
        if let Err(err) = net::handle_conn(stream, relay, ctx, events.clone(), inbound).await {
            let _ = events
                .send(NetEvent::Notice(format!("中继连接握手失败: {err}")))
                .await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn relay_message_roundtrip() {
        let (mut a, b) = tokio::io::duplex(4096);
        let msg = RelayMsg::Hello {
            room: "room".to_string(),
            pubkey: "pk".to_string(),
            addr: "1.2.3.4:5".to_string(),
        };
        write_msg(&mut a, &msg).await.unwrap();
        let mut b = b;
        let read = read_msg(&mut b).await.unwrap().unwrap();
        match read {
            RelayMsg::Hello { room, .. } => assert_eq!(room, "room"),
            _ => panic!("类型错误"),
        }
    }

    #[tokio::test]
    async fn relay_keeps_known_rooms_across_restart() {
        let path =
            std::env::temp_dir().join(format!("asa-relay-state-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);

        let state = RelayState::new(Some(path.clone()));
        state.known.lock().await.insert("room-a".to_string(), 1234);
        state.save_known().await;

        let restored = RelayState::new(Some(path.clone()));
        restored.load().await;
        assert_eq!(
            restored.known.lock().await.get("room-a").copied(),
            Some(1234)
        );
        let _ = std::fs::remove_file(&path);
    }
}

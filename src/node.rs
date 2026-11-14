//! 节点主循环：成员管理、服主选举、消息泛洪、文件传输。
//!
//! 设计要点：
//! - 全网状 TCP 连接，任何成员都能接收新成员（每个人都有群密钥）。
//! - 聊天消息带签名与加密，按 `id` 去重后泛洪，不依赖服主转发。
//! - 「服主」= 在场时间最早且仍连通的成员；退出后其余节点立即重新选举，
//!   聊天与文件传输不受影响。

use std::collections::{HashMap, HashSet, VecDeque};
use std::io::Write as _;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender as StdSender;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use ed25519_dalek::SigningKey;
use sha2::{Digest, Sha256};
use tokio::io::AsyncReadExt;
use tokio::net::TcpListener;
use tokio::sync::mpsc;

use crate::crypto::{self, RoomKey};
use crate::discovery::{Discovered, Discovery};
use crate::history::History;
use crate::net::{self, HandshakeCtx, NetEvent};
use crate::paths::RuntimeDirs;
use crate::proto::{self, Frame, Hello, MemberInfo, Payload, ReplyRef, SyncMessage};
use crate::relay::{self, RelayCmd};
use crate::room::RoomConfig;
use crate::util::{self, human_size, now_ms, short_id};

/// 文件分块大小。
pub const CHUNK_SIZE: usize = 32 * 1024;
/// 单条聊天消息上限（字符）。
pub const MAX_CHAT_CHARS: usize = 4000;
/// 在线判定：超过这个时间没收到任何帧就认为对方离线。
const PEER_TIMEOUT: Duration = Duration::from_secs(22);
/// 心跳间隔。
const HEARTBEAT: Duration = Duration::from_secs(4);
/// 服主成员表广播间隔。
const MEMBERS_INTERVAL: Duration = Duration::from_secs(8);
/// 文件请求等待接受的超时。
const OFFER_TIMEOUT: Duration = Duration::from_secs(120);
/// 接收文件时的空闲超时。
const RECV_IDLE_TIMEOUT: Duration = Duration::from_secs(90);

pub struct NodeConfig {
    pub signing: SigningKey,
    pub pubkey: String,
    pub nick: Arc<RwLock<String>>,
    pub room_id: String,
    pub room_key: RoomKey,
    pub room_created: u64,
    pub creator: String,
    /// 本节点加入群的时间（毫秒），用于服主排序。
    pub self_since: u64,
    pub bind: SocketAddr,
    /// 对外通告地址；默认自动探测局域网 IP。
    pub advertise: Option<SocketAddr>,
    /// 额外写入邀请码的入口地址（例如开启 EasyTier 时的局域网备用入口）。
    pub extra_entries: Vec<String>,
    /// 写入邀请码的 EasyTier 节点；为空时不携带 `&et=`。
    pub invite_et_nodes: Vec<String>,
    pub dirs: RuntimeDirs,
    pub identity_path: PathBuf,
    /// 接收文件的保存目录（可被全局/群配置覆盖）。
    pub downloads: PathBuf,
    pub max_file: u64,
    pub auto_accept: bool,
    pub mdns: bool,
    pub bootstrap: Vec<SocketAddr>,
    /// 加入频道后自动向在线成员拉取最近消息。
    pub sync_history: bool,
    /// 每次同步请求的条数；0 表示全部（最多 5000 条）。
    pub sync_count: u32,
    /// 启动时从本地 JSONL 加载并显示的历史条数；0 表示不加载。
    pub history_limit: usize,
    /// 中继服务器地址（可多个，当前使用第一个可解析的）。
    pub relays: Vec<String>,
}

/// 本机所有网卡的 IP；用于识别指向自身的拨号。
fn local_ip_addrs() -> Vec<IpAddr> {
    if_addrs::get_if_addrs()
        .map(|list| list.into_iter().map(|item| item.ip()).collect())
        .unwrap_or_default()
}

/// 是否指向本机监听端口（避免拨号自身导致“拒绝连接到自身”）。
fn is_self_target(own: SocketAddr, local: &[IpAddr], addr: SocketAddr) -> bool {
    addr.port() == own.port()
        && (addr.ip() == own.ip()
            || addr.ip().is_loopback()
            || addr.ip().is_unspecified()
            || local.contains(&addr.ip()))
}

#[derive(Debug)]
pub enum Cmd {
    Chat {
        text: String,
        reply: Option<ReplyRef>,
    },
    SetNick(String),
    /// 修改本机显示的频道名称（不广播）。
    SetRoomName(String),
    /// 向在线成员请求最近聊天记录。
    Sync,
    /// 发起 1:1 语音通话。
    Call {
        target: String,
    },
    /// 接听来电。
    AcceptCall {
        call_id: String,
    },
    /// 拒绝来电。
    RejectCall {
        call_id: String,
    },
    /// 挂断当前通话。
    HangupCall,
    /// 本地采集的一帧 PCM（20ms，16kHz 单声道）。
    AudioFrame {
        data: Vec<u8>,
    },
    /// 连接 `ip:port` 或 `host:port`。
    Connect(String),
    DialAddr(SocketAddr),
    SendFile {
        target: String,
        path: PathBuf,
    },
    SendFileMany {
        targets: Vec<String>,
        path: PathBuf,
    },
    OfferReady {
        target: String,
        path: PathBuf,
        name: String,
        size: u64,
        sha256: String,
    },
    Accept(String),
    Reject(String),
    OutgoingFinished {
        xfer: String,
        ok: bool,
        msg: String,
    },
    Notice(String),
    Quit,
}

#[derive(Clone, Debug)]
pub struct MemberView {
    pub pubkey: String,
    pub peer_id: String,
    pub nick: String,
    pub addr: Option<String>,
    pub host: bool,
    pub me: bool,
    pub presence: u64,
}

#[derive(Debug)]
pub enum NodeEvent {
    Chat {
        from: String,
        nick: String,
        text: String,
        ts: u64,
        mine: bool,
        /// 信封 id，用于回复引用。
        id: String,
        /// 被 @ 的成员公钥。
        mentions: Vec<String>,
        /// 回复引用。
        reply: Option<ReplyRef>,
        /// 是否为历史同步消息。
        history: bool,
    },
    System {
        text: String,
        ts: u64,
    },
    Members {
        members: Vec<MemberView>,
        host: Option<String>,
        me: String,
    },
    FileOffer {
        xfer: String,
        from: String,
        from_nick: String,
        name: String,
        size: u64,
    },
    FileProgress {
        xfer: String,
        name: String,
        done: u64,
        total: u64,
        outgoing: bool,
    },
    FileComplete {
        xfer: String,
        name: String,
        path: Option<PathBuf>,
        outgoing: bool,
        ok: bool,
        msg: String,
    },
    /// 呼叫已发出/对方正在响铃。
    CallRinging {
        peer: String,
        nick: String,
        call_id: String,
    },
    /// 收到来电，等待接听/拒绝。
    IncomingCall {
        peer: String,
        nick: String,
        call_id: String,
    },
    /// 通话已建立。
    CallAccepted {
        peer: String,
        nick: String,
        call_id: String,
    },
    /// 通话结束/被拒绝/挂断。
    CallEnded {
        peer: String,
        nick: String,
        call_id: String,
        reason: String,
    },
    /// 收到一帧 PCM 音频。
    AudioFrame {
        data: Vec<u8>,
    },
    /// 本机频道名称变更。
    RoomName(String),
    Info(String),
    Shutdown,
}

pub struct NodeHandle {
    pub cmd: mpsc::UnboundedSender<Cmd>,
    pub events: std::sync::mpsc::Receiver<NodeEvent>,
    pub listen: SocketAddr,
    pub advertise: SocketAddr,
    pub peer_id: String,
    pub pubkey: String,
    pub room_id: String,
    pub room_created: u64,
    pub creator: String,
    pub invite: String,
}

struct Peer {
    nick: String,
    listen: Option<SocketAddr>,
    presence: u64,
    conn: u64,
    tx: mpsc::Sender<Vec<u8>>,
    inbound: bool,
    addr: SocketAddr,
    last_seen: Instant,
}

struct Known {
    nick: String,
    addr: Option<SocketAddr>,
}

struct CallSession {
    id: String,
    peer: String,
    local_port: u16,
    remote_ip: Option<IpAddr>,
    remote_port: Option<u16>,
    socket: Arc<std::net::UdpSocket>,
    stop: Arc<AtomicBool>,
    recv: Option<std::thread::JoinHandle<()>>,
    key: [u8; 32],
    out_seq: u64,
    last_udp: Instant,
    tcp_fallback: bool,
    outgoing: bool,
    pending: bool,
}

impl Drop for CallSession {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn call_key(room: &RoomKey, call_id: &str, a: &str, b: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"asa-voice-v1");
    hasher.update(room);
    hasher.update(call_id.as_bytes());
    let (first, second) = if a < b { (a, b) } else { (b, a) };
    hasher.update(first.as_bytes());
    hasher.update(second.as_bytes());
    let digest = hasher.finalize();
    let mut key = [0u8; 32];
    key.copy_from_slice(&digest[..32]);
    key
}

fn voice_nonce(direction: u8, seq: u64) -> [u8; 12] {
    let mut nonce = [0u8; 12];
    nonce[0] = direction;
    nonce[4..].copy_from_slice(&seq.to_be_bytes());
    nonce
}

fn spawn_voice_recv(
    socket: Arc<std::net::UdpSocket>,
    stop: Arc<AtomicBool>,
    key: [u8; 32],
    direction: u8,
    net_tx: mpsc::Sender<NetEvent>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        let mut in_seq: Option<u64> = None;
        while !stop.load(Ordering::Relaxed) {
            match socket.recv_from(&mut buf) {
                Ok((len, _addr)) => {
                    if len < 8 + 16 {
                        continue;
                    }
                    let seq = u64::from_be_bytes(buf[..8].try_into().unwrap_or([0u8; 8]));
                    if in_seq.map(|last| seq <= last).unwrap_or(false) {
                        continue;
                    }
                    let nonce = voice_nonce(direction, seq);
                    if let Ok(plain) = crypto::open_nonce(&key, b"asa-voice", &nonce, &buf[8..len])
                    {
                        in_seq = Some(seq);
                        if plain.len() <= 4096 {
                            let _ = net_tx.try_send(NetEvent::Voice(plain));
                        }
                    }
                }
                Err(err)
                    if err.kind() == std::io::ErrorKind::WouldBlock
                        || err.kind() == std::io::ErrorKind::TimedOut => {}
                Err(_) => break,
            }
        }
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum OutState {
    Offering,
    Sending,
}

struct Outgoing {
    target: String,
    name: String,
    size: u64,
    path: PathBuf,
    created: Instant,
    state: OutState,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum InState {
    Pending,
    Receiving,
}

struct Incoming {
    from: String,
    name: String,
    size: u64,
    sha: String,
    last: Instant,
    state: InState,
    tmp_path: Option<PathBuf>,
    final_path: PathBuf,
    file: Option<std::fs::File>,
    hasher: Sha256,
    got: u64,
    next_idx: u32,
}

pub struct Node;

impl Node {
    pub async fn start(cfg: NodeConfig) -> Result<NodeHandle> {
        let listener = TcpListener::bind(cfg.bind)
            .await
            .with_context(|| format!("监听 {} 失败（端口被占用？试试 --port）", cfg.bind))?;
        let local = listener.local_addr()?;
        let advertise = cfg.advertise.unwrap_or_else(|| {
            let ip = if local.ip().is_unspecified() {
                util::lan_ipv4()
            } else {
                local.ip()
            };
            SocketAddr::new(ip, local.port())
        });

        let ctx = Arc::new(HandshakeCtx {
            room: cfg.room_id.clone(),
            key: cfg.room_key,
            signing: cfg.signing.clone(),
            pubkey: cfg.pubkey.clone(),
            nick: cfg.nick.clone(),
            listen: advertise,
            self_since: cfg.self_since,
        });

        let (ev_tx, ev_rx) = std::sync::mpsc::channel();
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
        let (net_tx, net_rx) = mpsc::channel(4096);

        tokio::spawn(net::accept_loop(listener, ctx.clone(), net_tx.clone()));

        // 同时连接多个中继：任何一个可用即可发现节点，避免单个中继成为中心。
        let mut relay_tx = Vec::new();
        for text in cfg.relays.iter().take(8) {
            match crate::room::resolve_addr(text) {
                Ok(addr) => {
                    relay_tx.push(relay::spawn_client(
                        addr,
                        cfg.room_id.clone(),
                        cfg.pubkey.clone(),
                        advertise.to_string(),
                        ctx.clone(),
                        net_tx.clone(),
                    ));
                }
                Err(err) => {
                    let _ = ev_tx.send(NodeEvent::Info(format!("中继地址无法解析 {text}: {err}")));
                }
            }
        }

        let mut discovery = None;
        let mut disc_rx = None;
        if cfg.mdns {
            let (tx, rx) = std::sync::mpsc::channel();
            match Discovery::start(
                &cfg.room_id,
                &cfg.pubkey,
                cfg.nick.clone(),
                advertise.port(),
                tx,
            ) {
                Ok(d) => {
                    discovery = Some(d);
                    disc_rx = Some(rx);
                }
                Err(err) => {
                    let _ = ev_tx.send(NodeEvent::Info(format!(
                        "mDNS 启动失败，局域网自动发现不可用: {err}"
                    )));
                }
            }
        }

        let mut room = RoomConfig::from_key(cfg.room_key, cfg.room_created, cfg.creator.clone());
        // 群入口持久化在 room.json 中；启动时自动尝试所有已知入口。
        if let Ok(Some(saved)) = crate::rooms::load(&cfg.dirs, &cfg.room_id) {
            if saved.id == room.id {
                room.entries = saved.entries;
                room.relays = saved.relays;
                room.et_nodes = saved.et_nodes;
            }
        }
        for entry in &cfg.extra_entries {
            let entry = entry.trim();
            if !entry.is_empty() && !room.entries.iter().any(|item| item == entry) {
                room.entries.push(entry.to_string());
            }
        }
        // 只有本次会话真正开启 EasyTier 时才把节点写进邀请码。
        room.et_nodes = cfg.invite_et_nodes.clone();
        // 邀请码同时携带全局配置里的中继地址。
        room.relays = cfg.relays.clone();
        let invite = room.invite(Some(advertise));
        let self_ips = local_ip_addrs();
        let mut bootstrap = cfg.bootstrap.clone();
        bootstrap.retain(|addr| !is_self_target(advertise, &self_ips, *addr));
        for entry in &room.entries {
            if let Ok(addr) = crate::room::resolve_addr(entry) {
                if !is_self_target(advertise, &self_ips, addr) && !bootstrap.contains(&addr) {
                    bootstrap.push(addr);
                }
            }
        }

        let _ = std::fs::create_dir_all(&cfg.downloads);
        let history = History::new(cfg.dirs.logs.clone(), cfg.room_id.clone())?;

        // 启动时把本地历史加载到界面，并填充同步缓存。
        // 旧日志没有完整公钥时，本机消息按短 ID 还原；其他成员仅在
        // 能匹配到完整公钥时才进入同步缓存。
        let self_short = short_id(&cfg.pubkey);
        let self_pubkey = cfg.pubkey.clone();
        let local_chats = if cfg.history_limit == 0 {
            Vec::new()
        } else {
            history.load_recent(cfg.history_limit, |from_id| {
                if from_id == self_short {
                    Some(self_pubkey.clone())
                } else {
                    None
                }
            })
        };
        let mut initial_recent = VecDeque::new();
        for chat in &local_chats {
            let from = chat.pubkey.clone().unwrap_or_else(|| chat.from_id.clone());
            let mine = from == cfg.pubkey;
            let _ = ev_tx.send(NodeEvent::Chat {
                from: from.clone(),
                nick: chat.from.clone(),
                text: chat.text.clone(),
                ts: chat.ts,
                mine,
                id: chat.id.clone(),
                mentions: chat.mentions.clone(),
                reply: chat.reply.clone(),
                history: true,
            });
            if let Some(pubkey) = &chat.pubkey {
                initial_recent.push_back(SyncMessage {
                    id: chat.id.clone(),
                    from: pubkey.clone(),
                    nick: chat.from.clone(),
                    text: chat.text.clone(),
                    ts: chat.ts,
                    mentions: chat.mentions.clone(),
                    reply: chat.reply.clone(),
                });
            }
        }
        while initial_recent.len() > 500 {
            initial_recent.pop_front();
        }

        let state = NodeState {
            ctx: ctx.clone(),
            room_created: cfg.room_created,
            creator: cfg.creator.clone(),
            self_since: cfg.self_since,
            identity_path: cfg.identity_path,
            dirs: cfg.dirs.clone(),
            peers: HashMap::new(),
            known: HashMap::new(),
            seen: HashSet::new(),
            seen_order: VecDeque::new(),
            host: None,
            history,
            ev: ev_tx.clone(),
            cmd_tx: cmd_tx.clone(),
            net_tx,
            downloads: cfg.downloads.clone(),
            tmp: cfg.dirs.tmp.clone(),
            max_file: cfg.max_file,
            auto_accept: cfg.auto_accept,
            outgoing: HashMap::new(),
            incoming: HashMap::new(),
            dialing: HashMap::new(),
            last_dial: HashMap::new(),
            bootstrap: bootstrap.clone(),
            last_presence: Instant::now() - HEARTBEAT,
            last_members: Instant::now() - MEMBERS_INTERVAL,
            last_reconnect: Instant::now(),
            discovery,
            relay_tx,
            relay_tried: HashMap::new(),
            self_ips,
            recent: initial_recent,
            sync_history: cfg.sync_history,
            sync_count: cfg.sync_count,
            call: None,
        };

        let handle = NodeHandle {
            cmd: cmd_tx,
            events: ev_rx,
            listen: local,
            advertise,
            peer_id: util::peer_id(&cfg.pubkey),
            pubkey: cfg.pubkey.clone(),
            room_id: cfg.room_id.clone(),
            room_created: cfg.room_created,
            creator: cfg.creator.clone(),
            invite,
        };

        tokio::spawn(async move {
            state.run(cmd_rx, net_rx, disc_rx).await;
        });

        Ok(handle)
    }
}

struct NodeState {
    ctx: Arc<HandshakeCtx>,
    room_created: u64,
    creator: String,
    self_since: u64,
    identity_path: PathBuf,
    dirs: RuntimeDirs,
    peers: HashMap<String, Peer>,
    known: HashMap<String, Known>,
    seen: HashSet<String>,
    seen_order: VecDeque<String>,
    host: Option<String>,
    history: History,
    ev: StdSender<NodeEvent>,
    cmd_tx: mpsc::UnboundedSender<Cmd>,
    net_tx: mpsc::Sender<NetEvent>,
    downloads: PathBuf,
    tmp: PathBuf,
    max_file: u64,
    auto_accept: bool,
    outgoing: HashMap<String, Outgoing>,
    incoming: HashMap<String, Incoming>,
    dialing: HashMap<SocketAddr, Instant>,
    last_dial: HashMap<SocketAddr, Instant>,
    bootstrap: Vec<SocketAddr>,
    last_presence: Instant,
    last_members: Instant,
    last_reconnect: Instant,
    discovery: Option<Discovery>,
    relay_tx: Vec<mpsc::Sender<RelayCmd>>,
    relay_tried: HashMap<String, Instant>,
    self_ips: Vec<IpAddr>,
    recent: VecDeque<SyncMessage>,
    sync_history: bool,
    sync_count: u32,
    call: Option<CallSession>,
}

impl NodeState {
    async fn run(
        mut self,
        mut cmd_rx: mpsc::UnboundedReceiver<Cmd>,
        mut net_rx: mpsc::Receiver<NetEvent>,
        disc_rx: Option<std::sync::mpsc::Receiver<Discovered>>,
    ) {
        self.info(format!(
            "已监听 {} · 频道 {} · 你是 {} ({})",
            self.ctx.listen,
            self.ctx.room,
            self.nick(),
            short_id(&self.ctx.pubkey)
        ));
        self.recompute_host();
        // 若本节点不是创建者、且还没连上任何成员，此时的「服主」只是本地视角，
        // 不应向用户宣称自己是服主（通常紧接着会收到真正的成员表）。
        let lone_non_creator = self.creator != self.ctx.pubkey && self.peers.is_empty();
        if self.is_host() && !lone_non_creator {
            self.system("频道已就绪。你是当前服主，把 /invite 得到的邀请码发给朋友即可加入频道。");
        } else {
            self.system("频道已就绪。");
        }
        self.emit_members();
        for addr in self.bootstrap.clone() {
            self.dial(addr);
        }

        let mut tick = tokio::time::interval(Duration::from_millis(500));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                biased;
                cmd = cmd_rx.recv() => {
                    match cmd {
                        Some(cmd) => {
                            if !self.on_cmd(cmd) {
                                break;
                            }
                        }
                        None => break,
                    }
                }
                ev = net_rx.recv() => {
                    match ev {
                        Some(ev) => self.on_net(ev),
                        None => break,
                    }
                }
                _ = tick.tick() => {
                    if let Some(rx) = disc_rx.as_ref() {
                        while let Ok(found) = rx.try_recv() {
                            self.on_discovered(found);
                        }
                    }
                    self.on_tick();
                }
            }
        }
        self.shutdown();
    }

    fn shutdown(&mut self) {
        self.end_call("退出");
        if let Ok(raw) = proto::encode_frame(&Frame::Bye {
            reason: "正常退出".to_string(),
        }) {
            for peer in self.peers.values() {
                let _ = peer.tx.try_send(raw.clone());
            }
        }
        let _ = self.ev.send(NodeEvent::Shutdown);
    }

    // ---------------------------------------------------------------- 命令

    fn on_cmd(&mut self, cmd: Cmd) -> bool {
        match cmd {
            Cmd::Chat { text, reply } => {
                let text = util::sanitize_text(&text);
                if text.is_empty() {
                    return true;
                }
                if text.chars().count() > MAX_CHAT_CHARS {
                    self.info(format!("消息太长（上限 {MAX_CHAT_CHARS} 字）"));
                    return true;
                }
                if self.peers.is_empty() {
                    self.info("当前没有其他在线成员，消息只写入本地记录");
                }
                self.send_chat(&text, reply);
            }
            Cmd::SetNick(nick) => self.set_nick(nick),
            Cmd::SetRoomName(name) => self.set_room_name(name),
            Cmd::Sync => self.request_history(),
            Cmd::Call { target } => self.start_call(&target),
            Cmd::AcceptCall { call_id } => self.accept_call(&call_id),
            Cmd::RejectCall { call_id } => self.reject_call(&call_id),
            Cmd::HangupCall => self.hangup_call("本机挂断"),
            Cmd::AudioFrame { data } => self.send_audio_frame(data),
            Cmd::Connect(text) => self.connect_text(&text),
            Cmd::DialAddr(addr) => {
                if is_self_target(self.ctx.listen, &self.self_ips, addr) {
                    self.info(format!("跳过本机地址: {addr}"));
                    return true;
                }
                if !self.bootstrap.contains(&addr) {
                    self.bootstrap.push(addr);
                }
                self.dial(addr);
            }
            Cmd::SendFile { target, path } => self.begin_send(target, path),
            Cmd::SendFileMany { targets, path } => {
                if targets.is_empty() {
                    self.info("没有可发送的目标成员");
                } else {
                    for target in targets {
                        self.begin_send(target, path.clone());
                    }
                }
            }
            Cmd::OfferReady {
                target,
                path,
                name,
                size,
                sha256,
            } => self.send_offer(target, path, name, size, sha256),
            Cmd::Accept(xfer) => {
                if let Err(err) = self.accept_incoming(&xfer) {
                    self.info(format!("接受文件失败: {err}"));
                }
            }
            Cmd::Reject(xfer) => self.reject_incoming(&xfer, "对方拒绝接收"),
            Cmd::OutgoingFinished { xfer, ok, msg } => {
                if let Some(out) = self.outgoing.remove(&xfer) {
                    let text = if ok {
                        format!("文件已发送: {} ({})", out.name, human_size(out.size))
                    } else {
                        format!("发送失败: {} · {msg}", out.name)
                    };
                    self.system(text);
                    let _ = self.ev.send(NodeEvent::FileComplete {
                        xfer,
                        name: out.name,
                        path: None,
                        outgoing: true,
                        ok,
                        msg,
                    });
                }
            }
            Cmd::Notice(text) => self.info(text),
            Cmd::Quit => {
                for tx in &self.relay_tx {
                    let _ = tx.try_send(RelayCmd::Stop);
                }
                return false;
            }
        }
        true
    }

    fn connect_text(&mut self, text: &str) {
        let mut target = text.trim().to_string();
        if target.is_empty() {
            self.info("用法: /connect <ip:port>");
            return;
        }
        // 只给了 IP 时补默认端口
        if !target.contains(':') {
            target = format!("{target}:{}", self.ctx.listen.port());
        }
        let cmd_tx = self.cmd_tx.clone();
        tokio::spawn(async move {
            match tokio::net::lookup_host(target.as_str()).await {
                Ok(addrs) => {
                    let mut any = false;
                    for addr in addrs {
                        if addr.is_ipv4() {
                            any = true;
                            let _ = cmd_tx.send(Cmd::DialAddr(addr));
                        }
                    }
                    if !any {
                        let _ = cmd_tx.send(Cmd::Notice(format!("地址没有可用的 IPv4: {target}")));
                    }
                }
                Err(err) => {
                    let _ = cmd_tx.send(Cmd::Notice(format!("解析地址失败: {err}")));
                }
            }
        });
    }

    fn set_nick(&mut self, nick: String) {
        let nick = util::sanitize_text(&nick);
        if nick.is_empty() {
            self.info("昵称不能为空");
            return;
        }
        let nick: String = nick.chars().take(24).collect();
        if let Ok(mut guard) = self.ctx.nick.write() {
            *guard = nick.clone();
        }
        let identity = crate::identity::Identity {
            signing: self.ctx.signing.clone(),
            created_at: 0,
        };
        if let Err(err) = identity.save(&self.identity_path, &nick) {
            self.info(format!("昵称已改，但保存失败: {err}"));
        }
        let mut profile =
            crate::config::GroupProfile::load(&self.dirs, &self.ctx.room).unwrap_or_default();
        profile.nick = Some(nick.clone());
        if let Err(err) = profile.save(&self.dirs, &self.ctx.room) {
            self.info(format!("频道昵称保存失败: {err}"));
        }
        if let Some(discovery) = &self.discovery {
            discovery.refresh();
        }
        self.system(format!("你改名为 {nick}"));
        self.send_presence_to_all();
        self.emit_members();
        if self.is_host() {
            self.broadcast_members();
        }
    }

    fn set_room_name(&mut self, name: String) {
        let name: String = util::sanitize_text(&name).chars().take(40).collect();
        if name.is_empty() {
            self.info("频道名称不能为空");
            return;
        }
        let mut profile =
            crate::config::GroupProfile::load(&self.dirs, &self.ctx.room).unwrap_or_default();
        profile.name = Some(name.clone());
        if let Err(err) = profile.save(&self.dirs, &self.ctx.room) {
            self.info(format!("频道名称保存失败: {err}"));
            return;
        }
        let _ = self.ev.send(NodeEvent::RoomName(name.clone()));
        self.info(format!("频道名称已改为 {name}（仅本机显示）"));
    }

    fn nick(&self) -> String {
        self.ctx
            .nick
            .read()
            .map(|n| n.clone())
            .unwrap_or_else(|_| "匿名".to_string())
    }

    fn nick_of(&self, pubkey: &str) -> String {
        if pubkey == self.ctx.pubkey {
            return self.nick();
        }
        if let Some(peer) = self.peers.get(pubkey) {
            return peer.nick.clone();
        }
        if let Some(known) = self.known.get(pubkey) {
            if !known.nick.is_empty() {
                return known.nick.clone();
            }
        }
        short_id(pubkey)
    }

    // ---------------------------------------------------------------- 网络事件

    fn on_net(&mut self, event: NetEvent) {
        match event {
            NetEvent::Up {
                conn,
                peer,
                hello,
                tx,
                inbound,
                addr,
            } => self.on_up(conn, peer, hello, tx, inbound, addr),
            NetEvent::Frame { peer, frame } => match frame {
                Frame::Envelope(env) => self.on_envelope(&peer, env),
                Frame::Bye { .. } => {}
                _ => {}
            },
            NetEvent::Down { conn, peer, reason } => self.on_down(conn, &peer, &reason),
            NetEvent::DialFailed { addr, reason } => {
                self.dialing.remove(&addr);
                let _ = reason;
            }
            NetEvent::RelayPeers(peers) => {
                if self.relay_tx.is_empty() {
                    return;
                }
                let now = Instant::now();
                for peer in peers {
                    if peer.pubkey == self.ctx.pubkey || self.peers.contains_key(&peer.pubkey) {
                        continue;
                    }
                    if self
                        .relay_tried
                        .get(&peer.pubkey)
                        .map(|last| now.duration_since(*last) < Duration::from_secs(20))
                        .unwrap_or(false)
                    {
                        continue;
                    }
                    self.relay_tried.insert(peer.pubkey.clone(), now);
                    // 同时向所有中继发起连接；哪个可用就用哪个。
                    for tx in &self.relay_tx {
                        let _ = tx.try_send(RelayCmd::Connect(peer.pubkey.clone()));
                    }
                }
            }
            NetEvent::Voice(data) => {
                if let Some(call) = self.call.as_mut() {
                    call.last_udp = Instant::now();
                    let _ = self.ev.send(NodeEvent::AudioFrame { data });
                }
            }
            NetEvent::Notice(text) => {
                // 握手阶段的失败提示，只放状态栏，不写入聊天记录。
                self.info(text);
            }
        }
    }

    fn on_up(
        &mut self,
        conn: u64,
        peer: String,
        hello: Hello,
        tx: mpsc::Sender<Vec<u8>>,
        inbound: bool,
        addr: SocketAddr,
    ) {
        self.dialing.remove(&addr);
        if peer == self.ctx.pubkey {
            return;
        }
        // 同时拨号会产生两条连接；只保留「公钥较小方发起」的那条。
        let canonical = inbound == (peer < self.ctx.pubkey);
        if let Some(existing) = self.peers.get(&peer) {
            let existing_canonical = existing.inbound == (peer < self.ctx.pubkey);
            if existing_canonical || !canonical {
                return;
            }
        }

        let is_new = !self.peers.contains_key(&peer);
        let listen = hello.listen.parse::<SocketAddr>().ok();
        let mut presence = hello.presence;
        if presence > now_ms() {
            presence = now_ms();
        }
        self.peers.insert(
            peer.clone(),
            Peer {
                nick: hello.nick.clone(),
                listen,
                presence,
                conn,
                tx,
                inbound,
                addr,
                last_seen: Instant::now(),
            },
        );
        self.known.insert(
            peer.clone(),
            Known {
                nick: hello.nick.clone(),
                addr: listen.or(Some(addr)),
            },
        );

        if is_new {
            self.system(format!("{} 上线了 ({})", hello.nick, short_id(&peer)));
        }
        self.emit_members();
        self.send_presence_to(&peer);
        self.send_members_to(&peer);
        if self.sync_history {
            let _ = self.send_to(
                &peer,
                &Payload::SyncRequest {
                    limit: self.sync_count,
                },
            );
        }
        self.recompute_host();
        if self.is_host() {
            self.broadcast_members();
        }
    }

    fn on_down(&mut self, conn: u64, peer: &str, reason: &str) {
        let stale = self.peers.get(peer).map(|p| p.conn != conn).unwrap_or(true);
        if stale {
            return;
        }
        if let Some(removed) = self.peers.remove(peer) {
            let _ = reason;
            self.system(format!("{} 离线了 ({})", removed.nick, short_id(peer)));
        }
        if self
            .call
            .as_ref()
            .map(|call| call.peer == peer)
            .unwrap_or(false)
        {
            self.end_call("对方离线");
        }
        self.emit_members();
        self.recompute_host();
        if self.is_host() {
            self.broadcast_members();
        }
    }

    // ---------------------------------------------------------------- 消息

    fn mark_seen(&mut self, id: &str) -> bool {
        if self.seen.contains(id) {
            return false;
        }
        self.seen.insert(id.to_string());
        self.seen_order.push_back(id.to_string());
        while self.seen_order.len() > 8192 {
            if let Some(old) = self.seen_order.pop_front() {
                self.seen.remove(&old);
            }
        }
        true
    }

    fn broadcast(&self, raw: &[u8], except: &[&str]) {
        for (pubkey, peer) in &self.peers {
            if except.contains(&pubkey.as_str()) {
                continue;
            }
            // 聊天消息很小；队列满时丢弃而不是阻塞整个节点。
            let _ = peer.tx.try_send(raw.to_vec());
        }
    }

    fn send_to(&self, pubkey: &str, payload: &Payload) -> Result<()> {
        let peer = self
            .peers
            .get(pubkey)
            .ok_or_else(|| anyhow!("对方不在线"))?;
        let raw = proto::build_envelope(
            &self.ctx.key,
            &self.ctx.signing,
            &self.ctx.pubkey,
            &self.ctx.room,
            payload,
        )?;
        peer.tx
            .try_send(raw)
            .map_err(|_| anyhow!("发送队列已满，请稍后重试"))?;
        Ok(())
    }

    fn send_chat(&mut self, text: &str, reply: Option<ReplyRef>) {
        let mentions = self.parse_mentions(text);
        let reply = reply.map(|mut r| {
            r.nick = util::sanitize_text(&r.nick);
            r.preview = util::sanitize_text(&r.preview).chars().take(120).collect();
            r
        });
        let payload = Payload::Chat {
            text: text.to_string(),
            mentions: mentions.clone(),
            reply: reply.clone(),
        };
        match proto::build_envelope_with_id(
            &self.ctx.key,
            &self.ctx.signing,
            &self.ctx.pubkey,
            &self.ctx.room,
            &payload,
        ) {
            Ok((id, raw)) => {
                // 标记本地消息 id，避免中继回传时重复显示。
                self.mark_seen(&id);
                self.broadcast(&raw, &[]);
                self.record_chat(
                    &self.ctx.pubkey.clone(),
                    &id,
                    text,
                    &mentions,
                    reply.as_ref(),
                );
            }
            Err(err) => self.info(format!("发送失败: {err}")),
        }
    }

    /// 解析文本中的 `@昵称` 与 `@所有人`，返回被提及成员的公钥。
    fn parse_mentions(&self, text: &str) -> Vec<String> {
        let members: Vec<(String, String)> = self
            .peers
            .iter()
            .map(|(pubkey, peer)| (peer.nick.clone(), pubkey.clone()))
            .collect();
        parse_mentions_in(text, &members)
    }

    fn record_chat(
        &mut self,
        from: &str,
        id: &str,
        text: &str,
        mentions: &[String],
        reply: Option<&ReplyRef>,
    ) {
        let nick = self.nick_of(from);
        self.record_chat_at(from, id, &nick, text, now_ms(), mentions, reply, false);
    }

    #[allow(clippy::too_many_arguments)]
    fn record_chat_at(
        &mut self,
        from: &str,
        id: &str,
        nick: &str,
        text: &str,
        ts: u64,
        mentions: &[String],
        reply: Option<&ReplyRef>,
        history: bool,
    ) {
        let mine = from == self.ctx.pubkey;
        self.history
            .append_chat_at(nick, &short_id(from), from, text, id, ts, reply, mentions);
        self.recent.push_back(SyncMessage {
            id: id.to_string(),
            from: from.to_string(),
            nick: nick.to_string(),
            text: text.to_string(),
            ts,
            mentions: mentions.to_vec(),
            reply: reply.cloned(),
        });
        while self.recent.len() > 500 {
            self.recent.pop_front();
        }
        let _ = self.ev.send(NodeEvent::Chat {
            from: from.to_string(),
            nick: nick.to_string(),
            text: text.to_string(),
            ts,
            mine,
            id: id.to_string(),
            mentions: mentions.to_vec(),
            reply: reply.cloned(),
            history,
        });
    }

    fn start_call(&mut self, target: &str) {
        if self.call.is_some() {
            self.info("当前已有通话");
            return;
        }
        let Some(peer) = self.peers.get(target) else {
            self.info("对方不在线，无法发起通话");
            return;
        };
        let listen = peer.listen;
        let call_id = hex::encode(crypto::random_bytes::<8>());
        let key = call_key(&self.ctx.key, &call_id, &self.ctx.pubkey, target);
        let socket = match std::net::UdpSocket::bind("0.0.0.0:0") {
            Ok(socket) => socket,
            Err(err) => {
                self.info(format!("创建语音 UDP 失败: {err}"));
                return;
            }
        };
        let _ = socket.set_read_timeout(Some(Duration::from_millis(200)));
        let port = socket.local_addr().map(|addr| addr.port()).unwrap_or(0);
        let socket = Arc::new(socket);
        let stop = Arc::new(AtomicBool::new(false));
        let recv = spawn_voice_recv(socket.clone(), stop.clone(), key, 2, self.net_tx.clone());
        let nick = self.nick_of(target);
        self.call = Some(CallSession {
            id: call_id.clone(),
            peer: target.to_string(),
            local_port: port,
            remote_ip: listen.map(|addr| addr.ip()),
            remote_port: None,
            socket,
            stop,
            recv: Some(recv),
            key,
            out_seq: 0,
            last_udp: Instant::now(),
            tcp_fallback: listen.is_none(),
            outgoing: true,
            pending: true,
        });
        if let Err(err) = self.send_to(
            target,
            &Payload::CallInvite {
                call_id: call_id.clone(),
                port,
            },
        ) {
            self.info(format!("发送通话邀请失败: {err}"));
            self.end_call("邀请发送失败");
            return;
        }
        let _ = self.ev.send(NodeEvent::CallRinging {
            peer: target.to_string(),
            nick,
            call_id,
        });
    }

    fn accept_call(&mut self, call_id: &str) {
        let Some(call) = self.call.as_mut() else {
            return;
        };
        if call.id != call_id || !call.pending {
            return;
        }
        let peer = call.peer.clone();
        let port = call.local_port;
        call.pending = false;
        call.last_udp = Instant::now();
        let nick = self.nick_of(&peer);
        if let Err(err) = self.send_to(
            &peer,
            &Payload::CallAccept {
                call_id: call_id.to_string(),
                port,
            },
        ) {
            self.info(format!("接听失败: {err}"));
            self.end_call("接听失败");
            return;
        }
        let _ = self.ev.send(NodeEvent::CallAccepted {
            peer,
            nick,
            call_id: call_id.to_string(),
        });
    }

    fn reject_call(&mut self, call_id: &str) {
        if let Some(call) = self.call.as_ref() {
            if call.id == call_id {
                let peer = call.peer.clone();
                let _ = self.send_to(
                    &peer,
                    &Payload::CallReject {
                        call_id: call_id.to_string(),
                        reason: "对方拒绝".to_string(),
                    },
                );
            }
        }
        self.end_call("已拒绝");
    }

    fn hangup_call(&mut self, reason: &str) {
        if let Some(call) = self.call.as_ref() {
            let peer = call.peer.clone();
            let _ = self.send_to(
                &peer,
                &Payload::CallHangup {
                    call_id: call.id.clone(),
                },
            );
        }
        self.end_call(reason);
    }

    fn send_audio_frame(&mut self, data: Vec<u8>) {
        if data.is_empty() {
            return;
        }
        let Some(call) = self.call.as_mut() else {
            return;
        };
        if call.pending {
            return;
        }
        let peer = call.peer.clone();
        let call_id = call.id.clone();
        let seq = call.out_seq;
        call.out_seq = call.out_seq.wrapping_add(1);
        let mut sent = false;
        if !call.tcp_fallback {
            if let (Some(ip), Some(port)) = (call.remote_ip, call.remote_port) {
                let direction = if call.outgoing { 1 } else { 2 };
                let nonce = voice_nonce(direction, seq);
                if let Ok(ciphertext) = crypto::seal_nonce(&call.key, b"asa-voice", &nonce, &data) {
                    let mut packet = Vec::with_capacity(8 + ciphertext.len());
                    packet.extend_from_slice(&seq.to_be_bytes());
                    packet.extend_from_slice(&ciphertext);
                    if call
                        .socket
                        .send_to(&packet, SocketAddr::new(ip, port))
                        .is_ok()
                    {
                        sent = true;
                    }
                }
            }
        }
        if !sent {
            call.tcp_fallback = true;
            let encoded = crypto::b64e(&data);
            if let Err(err) = self.send_to(
                &peer,
                &Payload::VoiceFrame {
                    call_id,
                    seq,
                    data: encoded,
                },
            ) {
                self.info(format!("语音帧发送失败: {err}"));
            }
        }
    }

    fn end_call(&mut self, reason: &str) {
        let Some(mut call) = self.call.take() else {
            return;
        };
        call.stop.store(true, Ordering::Relaxed);
        if let Some(recv) = call.recv.take() {
            let _ = recv.join();
        }
        let nick = self.nick_of(&call.peer);
        let _ = self.ev.send(NodeEvent::CallEnded {
            peer: call.peer.clone(),
            nick,
            call_id: call.id.clone(),
            reason: reason.to_string(),
        });
        self.info(format!("通话结束: {reason}"));
    }

    fn request_history(&mut self) {
        if self.peers.is_empty() {
            self.info("当前没有在线成员，无法更新聊天内容");
            return;
        }
        let limit = if self.sync_count == 0 {
            0
        } else {
            self.sync_count.clamp(1, 5000)
        };
        let peers: Vec<String> = self.peers.keys().cloned().collect();
        let mut sent = 0;
        for peer in peers {
            if self.send_to(&peer, &Payload::SyncRequest { limit }).is_ok() {
                sent += 1;
            }
        }
        let label = if limit == 0 {
            "全部".to_string()
        } else {
            limit.to_string()
        };
        self.info(format!("已向 {sent} 位成员请求最近 {label} 条消息"));
    }

    fn on_envelope(&mut self, conn_peer: &str, env: proto::Envelope) {
        if env.room != self.ctx.room {
            return;
        }
        if !self.mark_seen(&env.id) {
            return;
        }
        let from = env.sender.clone();
        let payload = match proto::open_envelope(&self.ctx.key, &self.ctx.room, &env) {
            Ok(payload) => payload,
            Err(_) => return,
        };
        if let Some(peer) = self.peers.get_mut(&from) {
            peer.last_seen = Instant::now();
        }

        let relay = matches!(&payload, Payload::Chat { .. } | Payload::Members { .. });
        match payload {
            Payload::Chat {
                text,
                mentions,
                reply,
            } => {
                let text = util::sanitize_text(&text);
                if text.is_empty() {
                    return;
                }
                let text: String = text.chars().take(MAX_CHAT_CHARS * 2).collect();
                let mentions: Vec<String> = mentions.into_iter().take(64).collect();
                let reply = reply.map(|mut r| {
                    r.id = r.id.chars().take(64).collect();
                    r.nick = util::sanitize_text(&r.nick);
                    r.preview = util::sanitize_text(&r.preview).chars().take(120).collect();
                    r
                });
                let nick = self.nick_of(&from);
                self.record_chat_at(
                    &from,
                    &env.id,
                    &nick,
                    &text,
                    env.ts,
                    &mentions,
                    reply.as_ref(),
                    false,
                );
            }
            Payload::SyncRequest { limit } => {
                let limit = if limit == 0 {
                    5000
                } else {
                    limit.clamp(1, 5000) as usize
                };
                let mut known: HashMap<String, String> = HashMap::new();
                known.insert(short_id(&self.ctx.pubkey), self.ctx.pubkey.clone());
                for pubkey in self.peers.keys() {
                    known.insert(short_id(pubkey), pubkey.clone());
                }
                for pubkey in self.known.keys() {
                    known.insert(short_id(pubkey), pubkey.clone());
                }
                let chats = self
                    .history
                    .load_recent(limit, |from_id| known.get(from_id).cloned());
                let mut messages: Vec<SyncMessage> = chats
                    .into_iter()
                    .filter_map(|chat| {
                        let from = chat.pubkey?;
                        Some(SyncMessage {
                            id: chat.id,
                            from,
                            nick: chat.from,
                            text: chat.text,
                            ts: chat.ts,
                            mentions: chat.mentions,
                            reply: chat.reply,
                        })
                    })
                    .collect();
                if messages.is_empty() {
                    messages = self.recent.iter().rev().take(limit).cloned().collect();
                    messages.reverse();
                }
                let _ = self.send_to(&from, &Payload::SyncBatch { messages });
            }
            Payload::SyncBatch { messages } => {
                let mut synced = 0usize;
                for message in messages.into_iter().take(5000) {
                    if message.from.len() != 64
                        || message.id.is_empty()
                        || !self.mark_seen(&message.id)
                    {
                        continue;
                    }
                    let text = util::sanitize_text(&message.text);
                    if text.is_empty() {
                        continue;
                    }
                    let text: String = text.chars().take(MAX_CHAT_CHARS * 2).collect();
                    let nick = util::sanitize_text(&message.nick)
                        .chars()
                        .take(24)
                        .collect::<String>();
                    let nick = if nick.is_empty() {
                        self.nick_of(&message.from)
                    } else {
                        nick
                    };
                    let mentions: Vec<String> = message.mentions.into_iter().take(64).collect();
                    let reply = message.reply.map(|mut r| {
                        r.id = r.id.chars().take(64).collect();
                        r.nick = util::sanitize_text(&r.nick);
                        r.preview = util::sanitize_text(&r.preview).chars().take(120).collect();
                        r
                    });
                    self.record_chat_at(
                        &message.from,
                        &message.id,
                        &nick,
                        &text,
                        message.ts,
                        &mentions,
                        reply.as_ref(),
                        true,
                    );
                    synced += 1;
                }
                if synced == 0 {
                    self.info("没有可同步的聊天记录");
                } else {
                    self.info(format!("已同步 {synced} 条聊天记录"));
                }
            }
            Payload::CallInvite { call_id, port } => {
                if self.call.is_some() {
                    let _ = self.send_to(
                        &from,
                        &Payload::CallReject {
                            call_id,
                            reason: "对方忙".to_string(),
                        },
                    );
                    return;
                }
                let key = call_key(&self.ctx.key, &call_id, &self.ctx.pubkey, &from);
                let socket = match std::net::UdpSocket::bind("0.0.0.0:0") {
                    Ok(socket) => socket,
                    Err(err) => {
                        self.info(format!("创建语音 UDP 失败: {err}"));
                        return;
                    }
                };
                let _ = socket.set_read_timeout(Some(Duration::from_millis(200)));
                let local_port = socket.local_addr().map(|addr| addr.port()).unwrap_or(0);
                let socket = Arc::new(socket);
                let stop = Arc::new(AtomicBool::new(false));
                let recv =
                    spawn_voice_recv(socket.clone(), stop.clone(), key, 1, self.net_tx.clone());
                let remote_ip = self
                    .peers
                    .get(&from)
                    .and_then(|peer| peer.listen)
                    .map(|addr| addr.ip());
                self.call = Some(CallSession {
                    id: call_id.clone(),
                    peer: from.clone(),
                    local_port,
                    remote_ip,
                    remote_port: Some(port),
                    socket,
                    stop,
                    recv: Some(recv),
                    key,
                    out_seq: 0,
                    last_udp: Instant::now(),
                    tcp_fallback: remote_ip.is_none(),
                    outgoing: false,
                    pending: true,
                });
                let nick = self.nick_of(&from);
                let _ = self.ev.send(NodeEvent::IncomingCall {
                    peer: from.clone(),
                    nick,
                    call_id,
                });
            }
            Payload::CallAccept { call_id, port } => {
                let mut accepted = false;
                if let Some(call) = self.call.as_mut() {
                    if call.id == call_id && call.outgoing && call.pending {
                        call.remote_port = Some(port);
                        call.pending = false;
                        call.last_udp = Instant::now();
                        accepted = true;
                    }
                }
                if accepted {
                    let peer = self
                        .call
                        .as_ref()
                        .map(|call| call.peer.clone())
                        .unwrap_or_default();
                    let nick = self.nick_of(&peer);
                    let _ = self.ev.send(NodeEvent::CallAccepted {
                        peer,
                        nick,
                        call_id,
                    });
                }
            }
            Payload::CallReject { call_id, reason } => {
                if self
                    .call
                    .as_ref()
                    .map(|call| call.id == call_id)
                    .unwrap_or(false)
                {
                    self.end_call(&format!("对方拒绝: {reason}"));
                }
            }
            Payload::CallHangup { call_id } => {
                if self
                    .call
                    .as_ref()
                    .map(|call| call.id == call_id)
                    .unwrap_or(false)
                {
                    self.end_call("对方挂断");
                }
            }
            Payload::VoiceFrame { call_id, seq, data } => {
                let _ = seq;
                if self
                    .call
                    .as_ref()
                    .map(|call| call.id == call_id)
                    .unwrap_or(false)
                {
                    if let Ok(bytes) = crypto::b64d(&data) {
                        if bytes.len() <= 4096 {
                            let _ = self.ev.send(NodeEvent::AudioFrame { data: bytes });
                        }
                    }
                }
            }
            Payload::Presence {
                nick,
                listen,
                presence,
            } => {
                let nick = util::sanitize_text(&nick);
                let mut renamed = None;
                if let Some(peer) = self.peers.get_mut(&from) {
                    if !nick.is_empty() && nick != peer.nick {
                        renamed = Some((peer.nick.clone(), nick.clone()));
                        peer.nick = nick.clone();
                    }
                    if let Ok(addr) = listen.parse::<SocketAddr>() {
                        peer.listen = Some(addr);
                    }
                    peer.presence = presence.min(now_ms());
                    peer.last_seen = Instant::now();
                }
                if let Some((old, new)) = renamed {
                    self.system(format!("{old} 改名为 {new}"));
                }
                self.recompute_host();
                self.emit_members();
            }
            Payload::Members { members } => {
                self.handle_members(&from, members);
            }
            Payload::FileOffer {
                xfer,
                name,
                size,
                sha256,
            } => self.handle_file_offer(&from, xfer, name, size, sha256),
            Payload::FileAccept { xfer } => self.handle_file_accept(&from, &xfer),
            Payload::FileReject { xfer, reason } => {
                if let Some(out) = self.outgoing.remove(&xfer) {
                    if out.target == from {
                        self.system(format!("对方拒绝接收 {}: {reason}", out.name));
                    }
                }
            }
            Payload::FileChunk {
                xfer,
                idx,
                total,
                data,
            } => self.handle_file_chunk(&from, &xfer, idx, total, &data),
            Payload::FileDone { xfer } => self.finish_incoming(&xfer),
            Payload::FileResult { xfer, ok, msg } => {
                let name = self.outgoing.get(&xfer).map(|out| out.name.clone());
                match (name, ok) {
                    (Some(name), true) => self.system(format!("{name} 已被对方校验通过")),
                    (Some(name), false) => self.system(format!("{name} 对方校验失败: {msg}")),
                    (None, false) => self.info(format!("对方报告文件校验失败: {msg}")),
                    (None, true) => {}
                }
            }
        }

        if relay {
            if let Ok(raw) = proto::encode_frame(&Frame::Envelope(env)) {
                // 排除转发来源与原作者，避免回声。
                self.broadcast(&raw, &[conn_peer, from.as_str()]);
            }
        }
    }

    fn handle_members(&mut self, _from: &str, members: Vec<MemberInfo>) {
        let mut changed = false;
        for member in members {
            if member.pubkey == self.ctx.pubkey || member.pubkey.len() != 64 {
                continue;
            }
            if let Some(peer) = self.peers.get_mut(&member.pubkey) {
                // 在线成员的信息以自己的 Presence 为准，避免被伪造的成员表覆盖昵称。
                if peer.listen.is_none() {
                    if let Ok(addr) = member.listen.parse::<SocketAddr>() {
                        peer.listen = Some(addr);
                    }
                }
                continue;
            }
            let addr = member.listen.parse::<SocketAddr>().ok();
            let known = self
                .known
                .entry(member.pubkey.clone())
                .or_insert_with(|| Known {
                    nick: member.nick.clone(),
                    addr,
                });
            if known.addr.is_none() {
                known.addr = addr;
                changed = true;
            }
            if known.nick.is_empty() {
                known.nick = member.nick.clone();
            }
            if let Some(addr) = known.addr {
                if self.ctx.pubkey < member.pubkey {
                    self.dial(addr);
                }
            }
        }
        if changed {
            self.emit_members();
        }
    }

    fn send_presence_to_all(&mut self) {
        let payload = Payload::Presence {
            nick: self.nick(),
            listen: self.ctx.listen.to_string(),
            presence: self.self_since,
        };
        if let Ok(raw) = proto::build_envelope(
            &self.ctx.key,
            &self.ctx.signing,
            &self.ctx.pubkey,
            &self.ctx.room,
            &payload,
        ) {
            self.broadcast(&raw, &[]);
        }
    }

    fn send_presence_to(&mut self, pubkey: &str) {
        let payload = Payload::Presence {
            nick: self.nick(),
            listen: self.ctx.listen.to_string(),
            presence: self.self_since,
        };
        let _ = self.send_to(pubkey, &payload);
    }

    fn members_payload(&self) -> Payload {
        let mut members = Vec::with_capacity(self.peers.len() + 1);
        members.push(MemberInfo {
            pubkey: self.ctx.pubkey.clone(),
            nick: self.nick(),
            listen: self.ctx.listen.to_string(),
            presence: self.self_since,
        });
        for (pubkey, peer) in &self.peers {
            members.push(MemberInfo {
                pubkey: pubkey.clone(),
                nick: peer.nick.clone(),
                listen: peer
                    .listen
                    .map(|a| a.to_string())
                    .unwrap_or_else(|| peer.addr.to_string()),
                presence: peer.presence,
            });
        }
        Payload::Members { members }
    }

    fn send_members_to(&mut self, pubkey: &str) {
        let payload = self.members_payload();
        let _ = self.send_to(pubkey, &payload);
    }

    fn broadcast_members(&mut self) {
        let payload = self.members_payload();
        if let Ok(raw) = proto::build_envelope(
            &self.ctx.key,
            &self.ctx.signing,
            &self.ctx.pubkey,
            &self.ctx.room,
            &payload,
        ) {
            self.broadcast(&raw, &[]);
            self.last_members = Instant::now();
        }
    }

    // ---------------------------------------------------------------- 服主

    fn effective_presence(&self, pubkey: &str, claimed: u64) -> u64 {
        let now = now_ms();
        let floor = if pubkey == self.creator {
            self.room_created
        } else {
            self.room_created.saturating_add(1)
        };
        if floor >= now {
            return now;
        }
        claimed.clamp(floor, now)
    }

    fn is_host(&self) -> bool {
        self.host.as_deref() == Some(self.ctx.pubkey.as_str())
    }

    fn recompute_host(&mut self) {
        let now = now_ms();
        let mut best = (
            self.effective_presence(&self.ctx.pubkey, self.self_since),
            self.ctx.pubkey.clone(),
            self.nick(),
        );
        let mut candidates: Vec<(u64, String, String)> = Vec::new();
        for (pubkey, peer) in &self.peers {
            let since = self.effective_presence(pubkey, peer.presence);
            candidates.push((since, pubkey.clone(), peer.nick.clone()));
        }
        for cand in candidates {
            if (cand.0, &cand.1) < (best.0, &best.1) {
                best = cand;
            }
        }
        let new_host = best.1.clone();
        if self.host.as_deref() == Some(new_host.as_str()) {
            return;
        }
        let old = self.host.clone();
        self.host = Some(new_host.clone());
        let _ = now;
        if new_host == self.ctx.pubkey {
            let lone_non_creator = self.creator != self.ctx.pubkey && self.peers.is_empty();
            if !(old.is_none() && lone_non_creator) {
                self.system(if old.is_none() {
                    "你已成为服主（负责成员表同步与新成员入口）".to_string()
                } else {
                    "原服主已离线，你成为新的服主".to_string()
                });
            }
        } else {
            let old_nick = old
                .as_deref()
                .map(|p| self.nick_of(p))
                .unwrap_or_else(|| "-".to_string());
            let _ = old_nick;
            self.system(format!("当前服主: {}", best.2));
        }
        self.emit_members();
        if new_host == self.ctx.pubkey {
            self.broadcast_members();
        }
    }

    fn emit_members(&self) {
        let host = self.host.clone();
        let mut members = Vec::with_capacity(self.peers.len() + 1);
        members.push(MemberView {
            pubkey: self.ctx.pubkey.clone(),
            peer_id: util::peer_id(&self.ctx.pubkey),
            nick: self.nick(),
            addr: Some(self.ctx.listen.to_string()),
            host: host.as_deref() == Some(self.ctx.pubkey.as_str()),
            me: true,
            presence: self.self_since,
        });
        for (pubkey, peer) in &self.peers {
            members.push(MemberView {
                pubkey: pubkey.clone(),
                peer_id: util::peer_id(pubkey),
                nick: peer.nick.clone(),
                addr: peer.listen.or(Some(peer.addr)).map(|a| a.to_string()),
                host: host.as_deref() == Some(pubkey.as_str()),
                me: false,
                presence: peer.presence,
            });
        }
        members.sort_by(|a, b| {
            a.presence
                .cmp(&b.presence)
                .then_with(|| a.nick.cmp(&b.nick))
        });
        let _ = self.ev.send(NodeEvent::Members {
            members,
            host: host.clone(),
            me: self.ctx.pubkey.clone(),
        });
    }

    // ---------------------------------------------------------------- 定时任务

    fn on_tick(&mut self) {
        if let Some(call) = self.call.as_mut() {
            if !call.pending
                && !call.tcp_fallback
                && call.last_udp.elapsed() > Duration::from_secs(2)
            {
                call.tcp_fallback = true;
                self.info("UDP 音频不可用，已回退 TCP");
            }
        }
        let now = Instant::now();
        self.relay_tried
            .retain(|_, tried| now.duration_since(*tried) < Duration::from_secs(60));
        if now.duration_since(self.last_presence) >= HEARTBEAT {
            self.last_presence = now;
            self.send_presence_to_all();
        }
        if self.is_host() && now.duration_since(self.last_members) >= MEMBERS_INTERVAL {
            self.broadcast_members();
        }

        if now.duration_since(self.last_reconnect) >= Duration::from_secs(12) {
            self.last_reconnect = now;
            for addr in self.bootstrap.clone() {
                self.dial(addr);
            }
            let mut to_dial = Vec::new();
            for (pubkey, known) in &self.known {
                if self.peers.contains_key(pubkey) || *pubkey == self.ctx.pubkey {
                    continue;
                }
                if let Some(addr) = known.addr {
                    if self.ctx.pubkey < *pubkey {
                        to_dial.push(addr);
                    }
                }
            }
            for addr in to_dial {
                self.dial(addr);
            }
        }

        // 清理超时连接
        let stale: Vec<String> = self
            .peers
            .iter()
            .filter(|(_, p)| now.duration_since(p.last_seen) > PEER_TIMEOUT)
            .map(|(pubkey, _)| pubkey.clone())
            .collect();
        for pubkey in stale {
            if let Some(removed) = self.peers.remove(&pubkey) {
                self.system(format!("{} 超时离线 ({})", removed.nick, short_id(&pubkey)));
            }
            self.emit_members();
            self.recompute_host();
            if self.is_host() {
                self.broadcast_members();
            }
        }

        self.dialing
            .retain(|_, when| now.duration_since(*when) < Duration::from_secs(15));

        // 文件传输超时
        let expired: Vec<String> = self
            .outgoing
            .iter()
            .filter(|(_, out)| {
                out.state == OutState::Offering && now.duration_since(out.created) > OFFER_TIMEOUT
            })
            .map(|(xfer, _)| xfer.clone())
            .collect();
        for xfer in expired {
            if let Some(out) = self.outgoing.remove(&xfer) {
                self.system(format!("文件请求超时（对方未响应）: {}", out.name));
            }
        }

        let idle: Vec<String> = self
            .incoming
            .iter()
            .filter(|(_, inc)| {
                inc.state == InState::Receiving && now.duration_since(inc.last) > RECV_IDLE_TIMEOUT
            })
            .map(|(xfer, _)| xfer.clone())
            .collect();
        for xfer in idle {
            if let Some(inc) = self.incoming.remove(&xfer) {
                if let Some(tmp) = &inc.tmp_path {
                    let _ = std::fs::remove_file(tmp);
                }
                self.system(format!("接收超时，已取消: {}", inc.name));
            }
        }
    }

    fn on_discovered(&mut self, found: Discovered) {
        if found.room != self.ctx.room || found.peer == self.ctx.pubkey {
            return;
        }
        self.known
            .entry(found.peer.clone())
            .or_insert_with(|| Known {
                nick: found.nick.clone(),
                addr: Some(found.addr),
            });
        if self.ctx.pubkey < found.peer {
            self.dial(found.addr);
        }
    }

    fn dial(&mut self, addr: SocketAddr) {
        if addr.port() == 0
            || addr.ip().is_unspecified()
            || addr == self.ctx.listen
            || is_self_target(self.ctx.listen, &self.self_ips, addr)
        {
            return;
        }
        if self
            .peers
            .values()
            .any(|peer| peer.addr == addr || peer.listen == Some(addr))
        {
            return;
        }
        let now = Instant::now();
        if let Some(when) = self.dialing.get(&addr) {
            if now.duration_since(*when) < Duration::from_secs(10) {
                return;
            }
        }
        if let Some(when) = self.last_dial.get(&addr) {
            if now.duration_since(*when) < Duration::from_secs(4) {
                return;
            }
        }
        self.dialing.insert(addr, now);
        self.last_dial.insert(addr, now);
        net::spawn_dial(addr, self.ctx.clone(), self.net_tx.clone());
    }

    // ---------------------------------------------------------------- 文件传输

    fn resolve_target(&self, needle: &str) -> Option<String> {
        let needle = needle.trim();
        if needle.is_empty() {
            return None;
        }
        if self.peers.contains_key(needle) {
            return Some(needle.to_string());
        }
        let lower = needle.to_lowercase();
        let mut hits: Vec<String> = Vec::new();
        for (pubkey, peer) in &self.peers {
            if peer.nick == needle
                || short_id(pubkey) == needle
                || util::peer_id(pubkey).starts_with(&lower)
                || pubkey.starts_with(&lower)
            {
                return Some(pubkey.clone());
            }
            if peer.nick.to_lowercase().contains(&lower) {
                hits.push(pubkey.clone());
            }
        }
        if hits.len() == 1 {
            Some(hits.remove(0))
        } else {
            None
        }
    }

    fn begin_send(&mut self, target: String, path: PathBuf) {
        let target_pub = match self.resolve_target(&target) {
            Some(pubkey) => pubkey,
            None => {
                self.info(format!("找不到在线成员: {target}（用 /peers 查看）"));
                return;
            }
        };
        let path = util::expand_tilde(&path.to_string_lossy());
        let meta = match std::fs::metadata(&path) {
            Ok(meta) => meta,
            Err(err) => {
                self.info(format!("无法读取文件 {}: {err}", path.display()));
                return;
            }
        };
        if !meta.is_file() {
            self.info("只能发送普通文件");
            return;
        }
        let size = meta.len();
        if size > self.max_file {
            self.info(format!(
                "文件太大: {}，单文件上限 {}（用 --max-file-mb 调整）",
                human_size(size),
                human_size(self.max_file)
            ));
            return;
        }
        let raw_name = path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "file".to_string());
        let name = util::sanitize_filename(&raw_name);
        let cmd_tx = self.cmd_tx.clone();
        let sha_path = path.clone();
        tokio::spawn(async move {
            let result = tokio::task::spawn_blocking(move || sha256_file(&sha_path)).await;
            match result {
                Ok(Ok(sha)) => {
                    let _ = cmd_tx.send(Cmd::OfferReady {
                        target: target_pub,
                        path,
                        name,
                        size,
                        sha256: sha,
                    });
                }
                Ok(Err(err)) => {
                    let _ = cmd_tx.send(Cmd::Notice(format!("计算文件校验值时失败: {err}")));
                }
                Err(err) => {
                    let _ = cmd_tx.send(Cmd::Notice(format!("计算文件校验值失败: {err}")));
                }
            }
        });
    }

    fn send_offer(
        &mut self,
        target: String,
        path: PathBuf,
        name: String,
        size: u64,
        sha256: String,
    ) {
        if !self.peers.contains_key(&target) {
            self.info("对方已离线，文件未发送");
            return;
        }
        let xfer = hex::encode(crypto::random_bytes::<8>());
        let payload = Payload::FileOffer {
            xfer: xfer.clone(),
            name: name.clone(),
            size,
            sha256,
        };
        if let Err(err) = self.send_to(&target, &payload) {
            self.info(format!("发送文件请求失败: {err}"));
            return;
        }
        self.outgoing.insert(
            xfer.clone(),
            Outgoing {
                target: target.clone(),
                name: name.clone(),
                size,
                path,
                created: Instant::now(),
                state: OutState::Offering,
            },
        );
        self.system(format!(
            "已向 {} 请求发送 {} ({})，等待对方 /accept {}",
            self.nick_of(&target),
            name,
            human_size(size),
            short_id(&xfer)
        ));
    }

    fn handle_file_offer(
        &mut self,
        from: &str,
        xfer: String,
        name: String,
        size: u64,
        sha: String,
    ) {
        if !self.peers.contains_key(from) || self.incoming.contains_key(&xfer) {
            return;
        }
        let name = util::sanitize_filename(&name);
        if size > self.max_file {
            let _ = self.send_to(
                from,
                &Payload::FileReject {
                    xfer,
                    reason: format!("文件超过上限 {}", human_size(self.max_file)),
                },
            );
            return;
        }
        if sha.len() != 64 {
            let _ = self.send_to(
                from,
                &Payload::FileReject {
                    xfer,
                    reason: "校验值格式错误".to_string(),
                },
            );
            return;
        }
        let final_path = util::unique_path(&self.downloads, &name);
        let from_nick = self.nick_of(from);
        self.incoming.insert(
            xfer.clone(),
            Incoming {
                from: from.to_string(),
                name: name.clone(),
                size,
                sha,
                last: Instant::now(),
                state: InState::Pending,
                tmp_path: None,
                final_path: final_path.clone(),
                file: None,
                hasher: Sha256::new(),
                got: 0,
                next_idx: 0,
            },
        );
        self.system(format!(
            "{} 想发文件 {} ({}) · 输入 /accept {} 或 /deny {}",
            from_nick,
            name,
            human_size(size),
            short_id(&xfer),
            short_id(&xfer)
        ));
        let _ = self.ev.send(NodeEvent::FileOffer {
            xfer: xfer.clone(),
            from: from.to_string(),
            from_nick,
            name,
            size,
        });
        if self.auto_accept {
            let _ = self.accept_incoming(&xfer);
        }
    }

    fn accept_incoming(&mut self, xfer: &str) -> Result<()> {
        let Some(inc) = self.incoming.get_mut(xfer) else {
            bail!("没有这个文件请求（可能是缩写不对）");
        };
        if inc.state != InState::Pending {
            bail!("该请求已处理");
        }
        let from = inc.from.clone();
        let name = inc.name.clone();
        let size = inc.size;
        let tmp_path = self.tmp.join(format!("{xfer}.part"));
        let file = std::fs::File::create(&tmp_path)
            .with_context(|| format!("创建临时文件失败: {}", tmp_path.display()))?;
        inc.file = Some(file);
        inc.tmp_path = Some(tmp_path);
        inc.state = InState::Receiving;
        inc.last = Instant::now();
        self.send_to(
            &from,
            &Payload::FileAccept {
                xfer: xfer.to_string(),
            },
        )?;
        self.system(format!("开始接收 {} ({})", name, human_size(size)));
        Ok(())
    }

    fn reject_incoming(&mut self, xfer: &str, reason: &str) {
        let Some(inc) = self.incoming.remove(xfer) else {
            self.info("没有这个文件请求");
            return;
        };
        let _ = self.send_to(
            &inc.from,
            &Payload::FileReject {
                xfer: xfer.to_string(),
                reason: reason.to_string(),
            },
        );
        if let Some(tmp) = &inc.tmp_path {
            let _ = std::fs::remove_file(tmp);
        }
        self.system(format!("已拒绝 {}: {reason}", inc.name));
    }

    fn handle_file_accept(&mut self, from: &str, xfer: &str) {
        let Some(out) = self.outgoing.get_mut(xfer) else {
            return;
        };
        if out.target != from || out.state != OutState::Offering {
            return;
        }
        out.state = OutState::Sending;
        let path = out.path.clone();
        let name = out.name.clone();
        let size = out.size;
        let target = out.target.clone();
        let Some(peer) = self.peers.get(&target) else {
            self.info("对方已离线，取消发送");
            self.outgoing.remove(xfer);
            return;
        };
        let tx = peer.tx.clone();
        let ctx = self.ctx.clone();
        let ev = self.ev.clone();
        let cmd_tx = self.cmd_tx.clone();
        let xfer = xfer.to_string();
        let task_xfer = xfer.clone();
        let task_name = name.clone();
        self.system(format!(
            "对方已接受，开始发送 {} ({})",
            name,
            human_size(size)
        ));
        tokio::spawn(async move {
            let result = send_file_task(ctx, tx, ev, &task_xfer, &path, size, &task_name).await;
            let (ok, msg) = match result {
                Ok(sent) => (true, format!("已发送 {} 字节", sent)),
                Err(err) => (false, err.to_string()),
            };
            let _ = cmd_tx.send(Cmd::OutgoingFinished { xfer, ok, msg });
        });
    }

    fn handle_file_chunk(&mut self, from: &str, xfer: &str, idx: u32, total: u32, data: &str) {
        let mut progress = None;
        let mut failure: Option<String> = None;
        if let Some(inc) = self.incoming.get_mut(xfer) {
            if inc.from != from || inc.state != InState::Receiving {
                return;
            }
            if idx != inc.next_idx {
                // TCP 保证顺序；乱序说明数据异常，直接终止。
                failure = Some("分块序号不连续".to_string());
            } else {
                match crypto::b64d(data) {
                    Ok(bytes) => {
                        let write_ok = match inc.file.as_mut() {
                            Some(file) => file.write_all(&bytes).is_ok(),
                            None => false,
                        };
                        if !write_ok {
                            failure = Some("写入磁盘失败".to_string());
                        } else {
                            inc.hasher.update(&bytes);
                            inc.got += bytes.len() as u64;
                            inc.next_idx += 1;
                            inc.last = Instant::now();
                            progress = Some((inc.name.clone(), inc.got, inc.size));
                            let _ = total;
                        }
                    }
                    Err(_) => failure = Some("分块编码错误".to_string()),
                }
            }
        } else {
            return;
        }

        if let Some((name, done, total)) = progress {
            let _ = self.ev.send(NodeEvent::FileProgress {
                xfer: xfer.to_string(),
                name,
                done,
                total,
                outgoing: false,
            });
        }
        if let Some(reason) = failure {
            if let Some(inc) = self.incoming.remove(xfer) {
                if let Some(tmp) = &inc.tmp_path {
                    let _ = std::fs::remove_file(tmp);
                }
                let _ = self.send_to(
                    from,
                    &Payload::FileResult {
                        xfer: xfer.to_string(),
                        ok: false,
                        msg: reason.clone(),
                    },
                );
                self.system(format!("接收失败: {} · {reason}", inc.name));
                let _ = self.ev.send(NodeEvent::FileComplete {
                    xfer: xfer.to_string(),
                    name: inc.name,
                    path: None,
                    outgoing: false,
                    ok: false,
                    msg: reason,
                });
            }
        }
    }

    fn finish_incoming(&mut self, xfer: &str) {
        let Some(inc) = self.incoming.remove(xfer) else {
            return;
        };
        let from = inc.from.clone();
        let name = inc.name.clone();
        let expected_size = inc.size;
        let expected_sha = inc.sha;
        let got = inc.got;
        let digest = hex::encode(inc.hasher.finalize());
        let tmp_path = inc.tmp_path.clone();
        let mut final_path = inc.final_path.clone();
        drop(inc.file);

        let mut ok = got == expected_size && digest == expected_sha;
        let mut msg = if ok {
            "接收完成".to_string()
        } else {
            format!("校验失败（收到 {got} / 期望 {expected_size} 字节）")
        };
        if ok {
            if let Some(tmp) = &tmp_path {
                match std::fs::rename(tmp, &final_path) {
                    Ok(_) => {}
                    Err(err) => {
                        ok = false;
                        msg = format!("保存文件失败: {err}");
                    }
                }
            } else {
                ok = false;
                msg = "临时文件丢失".to_string();
            }
        }
        if !ok {
            if let Some(tmp) = &tmp_path {
                let _ = std::fs::remove_file(tmp);
            }
            final_path = PathBuf::new();
        }

        let _ = self.send_to(
            &from,
            &Payload::FileResult {
                xfer: xfer.to_string(),
                ok,
                msg: msg.clone(),
            },
        );
        self.history.append(
            "file",
            &self.nick_of(&from),
            &short_id(&from),
            &format!("接收 {name} · {msg}"),
        );
        if ok {
            self.system(format!("文件已保存: {} → {}", name, final_path.display()));
        } else {
            self.system(format!("接收失败: {name} · {msg}"));
        }
        let _ = self.ev.send(NodeEvent::FileComplete {
            xfer: xfer.to_string(),
            name,
            path: if ok { Some(final_path) } else { None },
            outgoing: false,
            ok,
            msg,
        });
    }

    // ---------------------------------------------------------------- 提示

    fn info(&self, text: impl Into<String>) {
        let _ = self.ev.send(NodeEvent::Info(text.into()));
    }

    fn system(&mut self, text: impl Into<String>) {
        let text = text.into();
        self.history.append("system", "system", "", &text);
        let _ = self.ev.send(NodeEvent::System { text, ts: now_ms() });
    }
}

async fn send_file_task(
    ctx: Arc<HandshakeCtx>,
    tx: mpsc::Sender<Vec<u8>>,
    ev: StdSender<NodeEvent>,
    xfer: &str,
    path: &PathBuf,
    size: u64,
    name: &str,
) -> Result<u64> {
    let mut file = tokio::fs::File::open(path)
        .await
        .with_context(|| format!("打开文件失败: {}", path.display()))?;
    let total = ((size + CHUNK_SIZE as u64 - 1) / CHUNK_SIZE as u64) as u32;
    let mut buf = vec![0u8; CHUNK_SIZE];
    let mut idx = 0u32;
    let mut sent = 0u64;
    loop {
        let n = file.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        let payload = Payload::FileChunk {
            xfer: xfer.to_string(),
            idx,
            total,
            data: crypto::b64e(&buf[..n]),
        };
        let raw = proto::build_envelope(&ctx.key, &ctx.signing, &ctx.pubkey, &ctx.room, &payload)?;
        tx.send(raw).await.map_err(|_| anyhow!("连接已断开"))?;
        sent += n as u64;
        idx += 1;
        if idx % 8 == 0 {
            let _ = ev.send(NodeEvent::FileProgress {
                xfer: xfer.to_string(),
                name: name.to_string(),
                done: sent,
                total: size,
                outgoing: true,
            });
        }
    }
    let payload = Payload::FileDone {
        xfer: xfer.to_string(),
    };
    let raw = proto::build_envelope(&ctx.key, &ctx.signing, &ctx.pubkey, &ctx.room, &payload)?;
    tx.send(raw).await.map_err(|_| anyhow!("连接已断开"))?;
    let _ = ev.send(NodeEvent::FileProgress {
        xfer: xfer.to_string(),
        name: name.to_string(),
        done: sent,
        total: size,
        outgoing: true,
    });
    Ok(sent)
}

fn sha256_file(path: &std::path::Path) -> Result<String> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// 判断字符是否可视为昵称的一部分（用于确认 @昵称 的右边界）。
fn is_word_char(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_'
}

fn mention_boundary(next: Option<char>) -> bool {
    next.map(|ch| !is_word_char(ch)).unwrap_or(true)
}

fn starts_with_ignore_case(text: &str, prefix: &str) -> bool {
    let mut it = text.chars();
    for want in prefix.chars() {
        match it.next() {
            Some(got) if got.to_lowercase().eq(want.to_lowercase()) => {}
            _ => return false,
        }
    }
    true
}

fn push_unique(out: &mut Vec<String>, pubkey: &str) {
    if !out.iter().any(|item| item == pubkey) {
        out.push(pubkey.to_string());
    }
}

/// 从消息文本中解析 `@昵称` 与 `@所有人`，返回成员的 pubkey 列表。
///
/// - 昵称匹配不区分大小写，且优先匹配较长的昵称；
/// - `@` 前紧邻字母/数字时视为邮箱等场景，不解析；
/// - 昵称后必须是空白、标点或文本结尾。
fn parse_mentions_in(text: &str, members: &[(String, String)]) -> Vec<String> {
    const ALL: &str = "所有人";
    let mut sorted: Vec<&(String, String)> = members
        .iter()
        .filter(|(nick, _)| !nick.is_empty())
        .collect();
    sorted.sort_by_key(|(nick, _)| std::cmp::Reverse(nick.chars().count()));

    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '@' {
            i += 1;
            continue;
        }
        if i > 0 && is_word_char(chars[i - 1]) {
            i += 1;
            continue;
        }
        let rest: String = chars[i + 1..].iter().collect();
        if starts_with_ignore_case(&rest, ALL)
            && mention_boundary(chars.get(i + 1 + ALL.chars().count()).copied())
        {
            for (_, pubkey) in members {
                push_unique(&mut out, pubkey);
            }
            i += 1 + ALL.chars().count();
            continue;
        }
        let mut matched = false;
        for (nick, pubkey) in &sorted {
            if starts_with_ignore_case(&rest, nick) {
                let after = i + 1 + nick.chars().count();
                if mention_boundary(chars.get(after).copied()) {
                    push_unique(&mut out, pubkey);
                    i = after;
                    matched = true;
                    break;
                }
            }
        }
        if !matched {
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod mention_tests {
    use super::*;

    fn members() -> Vec<(String, String)> {
        vec![
            ("alice".to_string(), "pk-alice".to_string()),
            ("bob".to_string(), "pk-bob".to_string()),
            ("张三".to_string(), "pk-zhang".to_string()),
            ("张三丰".to_string(), "pk-zhangsf".to_string()),
        ]
    }

    #[test]
    fn parses_basic_mentions() {
        let got = parse_mentions_in("你好 @alice 和 @bob，看下", &members());
        assert_eq!(got, vec!["pk-alice", "pk-bob"]);
    }

    #[test]
    fn mention_is_case_insensitive() {
        let got = parse_mentions_in("@Alice 你好", &members());
        assert_eq!(got, vec!["pk-alice"]);
    }

    #[test]
    fn prefers_longer_nickname() {
        let got = parse_mentions_in("@张三丰 你好", &members());
        assert_eq!(got, vec!["pk-zhangsf"]);
        let got = parse_mentions_in("@张三 你好", &members());
        assert_eq!(got, vec!["pk-zhang"]);
    }

    #[test]
    fn ignores_email_like_text_and_prefix() {
        assert!(parse_mentions_in("mail a@bob.com", &members()).is_empty());
        assert!(parse_mentions_in("@bobby 你好", &members()).is_empty());
    }

    #[test]
    fn all_expands_to_everyone_once() {
        let got = parse_mentions_in("@所有人 开会，@alice 也来", &members());
        assert_eq!(got.len(), 4);
        assert!(got.contains(&"pk-alice".to_string()));
        let all_only = parse_mentions_in("@所有人 @所有人", &members());
        assert_eq!(all_only.len(), 4);
    }

    #[cfg(test)]
    mod self_target_tests {
        use super::*;

        #[test]
        fn detects_local_interfaces_and_loopback() {
            let own: SocketAddr = "192.168.1.10:7777".parse().unwrap();
            let local: Vec<IpAddr> = vec!["192.168.1.10".parse().unwrap()];
            assert!(is_self_target(
                own,
                &local,
                "192.168.1.10:7777".parse().unwrap()
            ));
            assert!(is_self_target(
                own,
                &local,
                "127.0.0.1:7777".parse().unwrap()
            ));
            assert!(is_self_target(own, &local, "0.0.0.0:7777".parse().unwrap()));
            assert!(!is_self_target(
                own,
                &local,
                "192.168.1.11:7777".parse().unwrap()
            ));
            assert!(!is_self_target(
                own,
                &local,
                "192.168.1.10:7778".parse().unwrap()
            ));
        }
    }
}

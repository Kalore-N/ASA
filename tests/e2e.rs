//! 端到端测试：两个节点互连、聊天、文件传输、服主交接。

use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant, SystemTime};

use asa::identity::Identity;
use asa::node::{Cmd, Node, NodeConfig, NodeEvent, NodeHandle};
use asa::paths::RuntimeDirs;
use asa::room::RoomConfig;

fn unique_root(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("test-data")
        .join(format!("{tag}-{}-{nanos}", std::process::id()))
}

fn identity_path(dirs: &RuntimeDirs) -> PathBuf {
    dirs.identity_path()
}

async fn start_node(
    root: &Path,
    nick: &str,
    identity: Identity,
    room: &RoomConfig,
    self_since: u64,
    max_file: u64,
) -> (NodeHandle, RuntimeDirs) {
    let dirs = RuntimeDirs::ensure(root).unwrap();
    let cfg = NodeConfig {
        signing: identity.signing.clone(),
        pubkey: identity.pubkey_hex(),
        nick: Arc::new(RwLock::new(nick.to_string())),
        room_id: room.id.clone(),
        room_key: room.key,
        room_created: room.created_at,
        creator: room.creator.clone(),
        self_since,
        bind: "127.0.0.1:0".parse().unwrap(),
        advertise: None,
        dirs: dirs.clone(),
        identity_path: identity_path(&dirs),
        downloads: dirs.downloads.clone(),
        max_file,
        auto_accept: false,
        mdns: false,
        bootstrap: Vec::new(),
        sync_history: false,
        sync_count: 50,
        relays: Vec::new(),
    };
    let handle = Node::start(cfg).await.unwrap();
    (handle, dirs)
}

async fn start_node_sync(
    root: &Path,
    nick: &str,
    identity: Identity,
    room: &RoomConfig,
    self_since: u64,
) -> (NodeHandle, RuntimeDirs) {
    let dirs = RuntimeDirs::ensure(root).unwrap();
    let cfg = NodeConfig {
        signing: identity.signing.clone(),
        pubkey: identity.pubkey_hex(),
        nick: Arc::new(RwLock::new(nick.to_string())),
        room_id: room.id.clone(),
        room_key: room.key,
        room_created: room.created_at,
        creator: room.creator.clone(),
        self_since,
        bind: "127.0.0.1:0".parse().unwrap(),
        advertise: None,
        dirs: dirs.clone(),
        identity_path: identity_path(&dirs),
        downloads: dirs.downloads.clone(),
        max_file: 8 * 1024 * 1024,
        auto_accept: false,
        mdns: false,
        bootstrap: Vec::new(),
        sync_history: true,
        sync_count: 50,
        relays: Vec::new(),
    };
    let handle = Node::start(cfg).await.unwrap();
    (handle, dirs)
}

/// 等待满足条件的事件；超时返回 false。
fn wait_for<F>(rx: &Receiver<NodeEvent>, timeout: Duration, mut pred: F) -> bool
where
    F: FnMut(&NodeEvent) -> bool,
{
    let deadline = Instant::now() + timeout;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return false;
        }
        match rx.recv_timeout(left) {
            Ok(event) => {
                if std::env::var_os("ASA_DEBUG").is_some() {
                    eprintln!("[event] {event:?}");
                }
                if pred(&event) {
                    return true;
                }
            }
            Err(_) => return false,
        }
    }
}

fn member_count(event: &NodeEvent) -> Option<usize> {
    match event {
        NodeEvent::Members { members, .. } => Some(members.len()),
        _ => None,
    }
}

fn is_chat_with(text: &str, mine: Option<bool>) -> impl FnMut(&NodeEvent) -> bool + '_ {
    move |event| match event {
        NodeEvent::Chat {
            text: got, mine: m, ..
        } => got == text && mine.map(|want| want == *m).unwrap_or(true),
        _ => false,
    }
}

fn connect(from: &NodeHandle, to: &NodeHandle) -> String {
    let addr = format!("127.0.0.1:{}", to.listen.port());
    from.cmd.send(Cmd::Connect(addr.clone())).unwrap();
    addr
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn chat_file_and_host_handover() {
    let root_a = unique_root("a");
    let root_b = unique_root("b");
    let identity_a = Identity::generate();
    let identity_b = Identity::generate();
    let room = RoomConfig::create(&identity_a);

    let (node_a, dirs_a) = start_node(
        &root_a,
        "alice",
        identity_a.clone(),
        &room,
        room.created_at,
        8 * 1024 * 1024,
    )
    .await;
    let (node_b, dirs_b) = start_node(
        &root_b,
        "bob",
        identity_b.clone(),
        &room,
        asa::util::now_ms(),
        8 * 1024 * 1024,
    )
    .await;

    connect(&node_b, &node_a);

    assert!(
        wait_for(&node_a.events, Duration::from_secs(8), |ev| {
            member_count(ev) == Some(2)
        }),
        "A 应该看到 2 个成员"
    );
    assert!(
        wait_for(&node_b.events, Duration::from_secs(8), |ev| {
            member_count(ev) == Some(2)
        }),
        "B 应该看到 2 个成员"
    );

    // 创建者（先在场）应当是服主
    let host_is_a = move |ev: &NodeEvent| match ev {
        NodeEvent::Members { host, .. } => {
            host.as_deref() == Some(identity_a.pubkey_hex().as_str())
        }
        _ => false,
    };
    assert!(
        wait_for(&node_b.events, Duration::from_secs(5), host_is_a),
        "B 应认为 A 是服主"
    );

    // 聊天双向可达
    node_a
        .cmd
        .send(Cmd::Chat {
            text: "来自 alice 的问候".into(),
            reply: None,
        })
        .unwrap();
    assert!(
        wait_for(
            &node_b.events,
            Duration::from_secs(8),
            is_chat_with("来自 alice 的问候", Some(false))
        ),
        "B 应收到 A 的消息"
    );
    node_b
        .cmd
        .send(Cmd::Chat {
            text: "bob 收到".into(),
            reply: None,
        })
        .unwrap();
    assert!(
        wait_for(
            &node_a.events,
            Duration::from_secs(8),
            is_chat_with("bob 收到", Some(false))
        ),
        "A 应收到 B 的回复"
    );

    // 文件传输：A -> B
    let payload: Vec<u8> = (0..120_000u32).map(|i| (i % 251) as u8).collect();
    let payload_path = root_a.join("payload.bin");
    std::fs::write(&payload_path, &payload).unwrap();
    node_a
        .cmd
        .send(Cmd::SendFile {
            target: "bob".into(),
            path: payload_path.clone(),
        })
        .unwrap();

    let mut offer_id = String::new();
    let got_offer = wait_for(&node_b.events, Duration::from_secs(10), |ev| {
        if let NodeEvent::FileOffer { xfer, size, .. } = ev {
            assert_eq!(*size, payload.len() as u64);
            offer_id = xfer.clone();
            true
        } else {
            false
        }
    });
    assert!(got_offer, "B 应收到文件请求");

    node_b.cmd.send(Cmd::Accept(offer_id.clone())).unwrap();

    let mut saved_path: Option<PathBuf> = None;
    let done = wait_for(&node_b.events, Duration::from_secs(30), |ev| {
        if let NodeEvent::FileComplete {
            ok,
            outgoing: false,
            path,
            ..
        } = ev
        {
            if *ok {
                saved_path = path.clone();
                return true;
            }
        }
        false
    });
    assert!(done, "B 应完成文件接收");
    let saved = saved_path.expect("应给出保存路径");
    let received = std::fs::read(&saved).unwrap();
    assert_eq!(received, payload, "接收内容应与原文件一致");
    assert!(saved.starts_with(&dirs_b.downloads));

    assert!(
        wait_for(&node_a.events, Duration::from_secs(10), |ev| {
            matches!(
                ev,
                NodeEvent::FileComplete {
                    outgoing: true,
                    ok: true,
                    ..
                }
            )
        }),
        "A 应收到发送完成事件"
    );

    // 原始服主退出，B 应成为新服主，聊天继续
    let pubkey_b = identity_b.pubkey_hex();
    node_a.cmd.send(Cmd::Quit).unwrap();
    assert!(
        wait_for(&node_b.events, Duration::from_secs(10), |ev| match ev {
            NodeEvent::Members { host, .. } => host.as_deref() == Some(pubkey_b.as_str()),
            _ => false,
        }),
        "A 退出后 B 应成为服主"
    );

    node_b
        .cmd
        .send(Cmd::Chat {
            text: "alice 走后我还在".into(),
            reply: None,
        })
        .unwrap();
    assert!(
        wait_for(
            &node_b.events,
            Duration::from_secs(5),
            is_chat_with("alice 走后我还在", Some(true))
        ),
        "服主退出后聊天不应中断"
    );

    // 本地日志应记录聊天
    let log_dir = dirs_b.logs.clone();
    let mut found = false;
    if let Ok(entries) = std::fs::read_dir(&log_dir) {
        for entry in entries.flatten() {
            if let Ok(text) = std::fs::read_to_string(entry.path()) {
                if text.contains("alice 走后我还在") && text.contains("来自 alice 的问候")
                {
                    found = true;
                }
            }
        }
    }
    assert!(found, "本地 JSONL 日志应包含聊天内容");

    node_b.cmd.send(Cmd::Quit).unwrap();
    let _ = (&node_a, &dirs_a);
    let _ = std::fs::remove_dir_all(&root_a);
    let _ = std::fs::remove_dir_all(&root_b);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn wrong_key_cannot_join() {
    let root_a = unique_root("ka");
    let root_c = unique_root("kc");
    let identity_a = Identity::generate();
    let identity_c = Identity::generate();
    let room = RoomConfig::create(&identity_a);
    let other_room = RoomConfig::create(&identity_c);

    let (node_a, dirs_a) = start_node(
        &root_a,
        "alice",
        identity_a.clone(),
        &room,
        room.created_at,
        1024 * 1024,
    )
    .await;
    // C 的群密钥不同，握手必须失败
    let (node_c, dirs_c) = start_node(
        &root_c,
        "mallory",
        identity_c.clone(),
        &other_room,
        asa::util::now_ms(),
        1024 * 1024,
    )
    .await;
    connect(&node_c, &node_a);

    // 给握手留出时间，然后确认双方都没有把对方当作成员
    std::thread::sleep(Duration::from_secs(2));
    let count_a = drain_members(&node_a.events);
    let count_c = drain_members(&node_c.events);
    assert_eq!(count_a, Some(1), "A 不应接受密钥错误的节点");
    assert_eq!(count_c, Some(1), "C 不应连上密钥不同的群");

    node_a.cmd.send(Cmd::Quit).unwrap();
    node_c.cmd.send(Cmd::Quit).unwrap();
    let _ = (&dirs_a, &dirs_c);
    let _ = std::fs::remove_dir_all(&root_a);
    let _ = std::fs::remove_dir_all(&root_c);
}

fn drain_members(rx: &Receiver<NodeEvent>) -> Option<usize> {
    let mut last = None;
    while let Ok(event) = rx.try_recv() {
        if let Some(count) = member_count(&event) {
            last = Some(count);
        }
    }
    last
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn host_order_follows_presence() {
    let root_a = unique_root("oa");
    let root_b = unique_root("ob");
    let root_c = unique_root("oc");
    let identity_a = Identity::generate();
    let identity_b = Identity::generate();
    let identity_c = Identity::generate();
    let room = RoomConfig::create(&identity_a);

    let (node_a, dirs_a) = start_node(
        &root_a,
        "alice",
        identity_a.clone(),
        &room,
        room.created_at,
        1024 * 1024,
    )
    .await;
    // 明确让 B 比 C 早入场 100ms
    let (node_b, dirs_b) = start_node(
        &root_b,
        "bob",
        identity_b.clone(),
        &room,
        room.created_at + 100,
        1024 * 1024,
    )
    .await;
    let (node_c, dirs_c) = start_node(
        &root_c,
        "carol",
        identity_c.clone(),
        &room,
        room.created_at + 200,
        1024 * 1024,
    )
    .await;

    connect(&node_b, &node_a);
    connect(&node_c, &node_a);

    assert!(
        wait_for(&node_c.events, Duration::from_secs(10), |ev| {
            member_count(ev) == Some(3)
        }),
        "C 应看到 3 个成员"
    );
    let host_a = identity_a.pubkey_hex();
    assert!(
        wait_for(&node_c.events, Duration::from_secs(5), move |ev| match ev {
            NodeEvent::Members { host, .. } => host.as_deref() == Some(host_a.as_str()),
            _ => false,
        }),
        "C 应认为创建者 A 是服主"
    );

    let host_b = identity_b.pubkey_hex();
    node_a.cmd.send(Cmd::Quit).unwrap();
    assert!(
        wait_for(
            &node_c.events,
            Duration::from_secs(10),
            move |ev| match ev {
                NodeEvent::Members { host, .. } => host.as_deref() == Some(host_b.as_str()),
                _ => false,
            }
        ),
        "A 退出后应由更早入场的 B 接任服主，而不是 C"
    );

    for node in [&node_b, &node_c] {
        node.cmd.send(Cmd::Quit).unwrap();
    }
    let _ = (&dirs_a, &dirs_b, &dirs_c);
    let _ = std::fs::remove_dir_all(&root_a);
    let _ = std::fs::remove_dir_all(&root_b);
    let _ = std::fs::remove_dir_all(&root_c);
}

/// 等待并返回第一个满足条件的事件。
fn wait_event<F>(rx: &Receiver<NodeEvent>, timeout: Duration, mut pred: F) -> Option<NodeEvent>
where
    F: FnMut(&NodeEvent) -> bool,
{
    let deadline = Instant::now() + timeout;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return None;
        }
        match rx.recv_timeout(left) {
            Ok(event) => {
                if pred(&event) {
                    return Some(event);
                }
            }
            Err(_) => return None,
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mention_and_reply_roundtrip() {
    let root_a = unique_root("mention-a");
    let root_b = unique_root("mention-b");
    let identity_a = Identity::generate();
    let identity_b = Identity::generate();
    let room = RoomConfig::create(&identity_a);

    let (node_a, _dirs_a) = start_node(
        &root_a,
        "alice",
        identity_a.clone(),
        &room,
        room.created_at,
        8 * 1024 * 1024,
    )
    .await;
    let (node_b, _dirs_b) = start_node(
        &root_b,
        "bob",
        identity_b.clone(),
        &room,
        asa::util::now_ms(),
        8 * 1024 * 1024,
    )
    .await;
    connect(&node_b, &node_a);
    assert!(
        wait_for(&node_b.events, Duration::from_secs(8), |ev| {
            member_count(ev) == Some(2)
        }),
        "B 应该看到 2 个成员"
    );

    // A 发送 @bob，B 应收到 mentions 中含自己公钥的消息。
    node_a
        .cmd
        .send(Cmd::Chat {
            text: "@bob 请查看".into(),
            reply: None,
        })
        .unwrap();
    let event = wait_event(&node_b.events, Duration::from_secs(8), |ev| {
        matches!(
            ev,
            NodeEvent::Chat {
                text,
                mine: false,
                ..
            } if text == "@bob 请查看"
        )
    })
    .expect("B 应收到提及消息");
    let (msg_id, mentions) = match event {
        NodeEvent::Chat { id, mentions, .. } => (id, mentions),
        _ => unreachable!(),
    };
    assert_eq!(mentions, vec![identity_b.pubkey_hex()]);

    // B 引用该消息回复，A 应收到 reply.id 指向原消息。
    let reply = asa::proto::ReplyRef {
        id: msg_id.clone(),
        nick: "alice".to_string(),
        preview: "@bob 请查看".to_string(),
    };
    node_b
        .cmd
        .send(Cmd::Chat {
            text: "收到".into(),
            reply: Some(reply),
        })
        .unwrap();
    let event = wait_event(&node_a.events, Duration::from_secs(8), |ev| {
        matches!(
            ev,
            NodeEvent::Chat {
                text,
                mine: false,
                ..
            } if text == "收到"
        )
    })
    .expect("A 应收到回复");
    match event {
        NodeEvent::Chat { reply, .. } => {
            let reply = reply.expect("回复应带引用");
            assert_eq!(reply.id, msg_id);
            assert_eq!(reply.nick, "alice");
            assert_eq!(reply.preview, "@bob 请查看");
        }
        _ => unreachable!(),
    }

    node_a.cmd.send(Cmd::Quit).ok();
    node_b.cmd.send(Cmd::Quit).ok();
}

async fn start_node_relay(
    root: &Path,
    nick: &str,
    identity: Identity,
    room: &RoomConfig,
    self_since: u64,
    relays: Vec<String>,
) -> (NodeHandle, RuntimeDirs) {
    let dirs = RuntimeDirs::ensure(root).unwrap();
    let cfg = NodeConfig {
        signing: identity.signing.clone(),
        pubkey: identity.pubkey_hex(),
        nick: Arc::new(RwLock::new(nick.to_string())),
        room_id: room.id.clone(),
        room_key: room.key,
        room_created: room.created_at,
        creator: room.creator.clone(),
        self_since,
        bind: "127.0.0.1:0".parse().unwrap(),
        advertise: None,
        dirs: dirs.clone(),
        identity_path: identity_path(&dirs),
        downloads: dirs.downloads.clone(),
        max_file: 8 * 1024 * 1024,
        auto_accept: false,
        mdns: false,
        bootstrap: Vec::new(),
        sync_history: false,
        sync_count: 50,
        relays,
    };
    let handle = Node::start(cfg).await.unwrap();
    (handle, dirs)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn relay_connects_two_nodes_without_direct_address() {
    let probe = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let relay_addr = probe.local_addr().unwrap();
    drop(probe);
    tokio::spawn(async move {
        let _ = asa::relay::run_relay(relay_addr, None).await;
    });
    let probe2 = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let relay2_addr = probe2.local_addr().unwrap();
    drop(probe2);
    tokio::spawn(async move {
        let _ = asa::relay::run_relay(relay2_addr, None).await;
    });
    tokio::time::sleep(Duration::from_millis(200)).await;
    let relays = vec![
        relay_addr.to_string(),
        relay2_addr.to_string(),
        "127.0.0.1:1".to_string(),
    ];

    let root_a = unique_root("relay-a");
    let root_b = unique_root("relay-b");
    let identity_a = Identity::generate();
    let identity_b = Identity::generate();
    let room = RoomConfig::create(&identity_a);

    let (node_a, _dirs_a) = start_node_relay(
        &root_a,
        "alice",
        identity_a.clone(),
        &room,
        room.created_at,
        relays.clone(),
    )
    .await;
    let (node_b, _dirs_b) = start_node_relay(
        &root_b,
        "bob",
        identity_b.clone(),
        &room,
        asa::util::now_ms(),
        relays,
    )
    .await;

    assert!(
        wait_for(&node_a.events, Duration::from_secs(12), |ev| {
            member_count(ev) == Some(2)
        }),
        "A 应通过中继看到 2 个成员"
    );
    assert!(
        wait_for(&node_b.events, Duration::from_secs(12), |ev| {
            member_count(ev) == Some(2)
        }),
        "B 应通过中继看到 2 个成员"
    );

    node_a
        .cmd
        .send(Cmd::Chat {
            text: "relay hello".into(),
            reply: None,
        })
        .unwrap();
    assert!(
        wait_for(
            &node_b.events,
            Duration::from_secs(10),
            is_chat_with("relay hello", Some(false))
        ),
        "B 应通过中继收到消息"
    );

    node_a.cmd.send(Cmd::Quit).ok();
    node_b.cmd.send(Cmd::Quit).ok();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sync_pulls_recent_history_from_peer() {
    let root_a = unique_root("sync-a");
    let root_b = unique_root("sync-b");
    let identity_a = Identity::generate();
    let identity_b = Identity::generate();
    let room = RoomConfig::create(&identity_a);

    let (node_a, _dirs_a) =
        start_node_sync(&root_a, "alice", identity_a.clone(), &room, room.created_at).await;
    node_a
        .cmd
        .send(Cmd::Chat {
            text: "before bob joined".into(),
            reply: None,
        })
        .unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;

    let (node_b, _dirs_b) = start_node_sync(
        &root_b,
        "bob",
        identity_b.clone(),
        &room,
        asa::util::now_ms(),
    )
    .await;
    connect(&node_b, &node_a);

    let event = wait_event(&node_b.events, Duration::from_secs(10), |ev| {
        matches!(
            ev,
            NodeEvent::Chat {
                text,
                history: true,
                ..
            } if text == "before bob joined"
        )
    });
    assert!(event.is_some(), "B 应从 A 同步到加入前的消息");

    node_a.cmd.send(Cmd::Quit).ok();
    node_b.cmd.send(Cmd::Quit).ok();
}

//! ASA 命令行入口。

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};

use asa::config::{Config, GroupProfile};
use asa::easytier::{self, EtProcess, EtStatus};
use asa::lan::JoinServer;
use asa::node::{Node, NodeConfig};
use asa::paths::{self, RuntimeDirs};
use asa::room::{self, RoomConfig};
use asa::settings::Settings;
use asa::tui::{self, TuiAction, UiConfig};
use asa::{identity::Identity, util};

const USAGE: &str = "\
ASA · 去中心化 TUI 频道

用法:
  asa init [--name 昵称] [--port 7777]     创建本地身份与频道，打印邀请码
  asa join <邀请码> [--name 昵称]          用邀请码加入频道
  asa chat [选项]                          启动聊天（默认命令）
  asa groups                               列出本地保存的频道
  asa invite [--port 7777]                 打印当前频道的邀请码
  asa entry add|remove <地址>              维护群入口地址
  asa entry list                           列出入口地址与长期邀请码
  asa relay serve [端口] [状态文件]        启动跨网中继（需公网主机）
  asa relay add|remove <地址>              维护中继地址
  asa relay list                           列出中继地址
  asa export <文件>                        导出群文件（含群密钥，注意保密）
  asa import <文件>                        导入群文件，恢复群
  asa help | --version

通用选项:
  --name <昵称>        指定昵称（会覆盖已保存的昵称）
  --port <端口>        TCP 监听端口，默认 7777
  --connect <地址>     手动连接成员，可重复，如 --connect 192.168.1.8:7777
  --no-mdns            关闭局域网自动发现
  --data-dir <路径>    数据目录，默认 ~/Documents/ASA
  --max-file-mb <MB>   单文件上限，默认 100，0 表示不限制
  --auto-accept        自动接受所有文件请求（有风险，慎用）
  --room <群ID>        指定要进入的本地频道（默认最近使用）

进入聊天后输入 /help 查看命令。
";

#[derive(Default)]
struct Opts {
    command: Option<String>,
    invite: Option<String>,
    name: Option<String>,
    port: Option<u16>,
    connect: Vec<String>,
    no_mdns: bool,
    data_root: Option<PathBuf>,
    max_file_mb: Option<u64>,
    auto_accept: bool,
    room: Option<String>,
    args: Vec<String>,
}

impl Opts {
    fn port(&self) -> u16 {
        self.port.unwrap_or(7777)
    }
}

fn main() {
    if let Err(err) = real_main() {
        eprintln!("错误: {err:#}");
        std::process::exit(1);
    }
}

fn real_main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let opts = parse_args(args)?;
    let command = opts.command.clone().unwrap_or_else(|| "chat".to_string());

    if command == "help" {
        println!("{USAGE}");
        return Ok(());
    }
    if command == "version" {
        println!("asa {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    let root = match &opts.data_root {
        Some(path) => path.clone(),
        None => paths::default_root()?,
    };
    let dirs = RuntimeDirs::ensure(&root)?;
    let mut config = Config::ensure(&dirs)?;
    let settings = Settings::load(&dirs.settings_path()).unwrap_or_default();
    let mut opts = opts;
    if opts.port.is_none() {
        opts.port = config.network.port.or(settings.port);
    }
    if !opts.auto_accept {
        opts.auto_accept = config.auto_accept;
    }
    if !opts.no_mdns {
        opts.no_mdns = !config.network.mdns;
    }
    asa::rooms::migrate(&dirs)?;
    let port = opts.port();
    let _ = Settings { port: Some(port) }.save(&dirs.settings_path());
    if config.network.port != Some(port) {
        config.network.port = Some(port);
        let _ = config.save(&asa::config::config_path(&dirs));
    }

    match command.as_str() {
        "init" => {
            let (identity, _) = Identity::load_or_create(&dirs.identity_path())?;
            let nick = opts.name.clone().unwrap_or_else(|| default_nick(&identity));
            identity.save(&dirs.identity_path(), &nick)?;
            let mut room = match asa::rooms::active(&dirs)? {
                Some(room) => room,
                None => RoomConfig::create(&identity),
            };
            merge_relays(&mut room, &config);
            asa::rooms::save(&dirs, &room)?;
            let addr = SocketAddr::new(util::lan_ipv4(), opts.port());
            if let Ok(mut profile) = GroupProfile::load(&dirs, &room.id) {
                if profile.nick.is_none() {
                    profile.nick = Some(nick.clone());
                    let _ = profile.save(&dirs, &room.id);
                }
            }
            println!("数据目录: {}", dirs.root.display());
            println!("配置文件: {}", asa::config::config_path(&dirs).display());
            println!("你的昵称: {nick}（ID {}）", identity.short());
            println!("群 ID: {}", room.id);
            println!("邀请码:");
            println!("  {}", room.invite(Some(addr)));
            println!();
            println!("本机启动聊天: asa");
            println!("让朋友运行:  asa join '<上面的邀请码>' --name 他的名字");
            println!("跨网时把邀请码里的地址换成你的公网/可达地址，或让对方用 /connect 手动连接。");
            Ok(())
        }
        "invite" => {
            let mut room = load_room(&dirs, &opts)?;
            merge_relays(&mut room, &config);
            let addr = SocketAddr::new(util::lan_ipv4(), opts.port());
            println!("{}", room.invite(Some(addr)));
            Ok(())
        }
        "entry" => {
            let mut room = load_room(&dirs, &opts)?;
            let action = opts.args.first().map(String::as_str).unwrap_or("list");
            match action {
                "add" => {
                    let value = opts
                        .args
                        .get(1)
                        .ok_or_else(|| anyhow!("用法: asa entry add <地址>"))?;
                    if !room.entries.iter().any(|entry| entry == value) {
                        room.entries.push(value.clone());
                    }
                    asa::rooms::save(&dirs, &room)?;
                    println!("已添加入口: {value}");
                }
                "remove" | "rm" => {
                    let value = opts
                        .args
                        .get(1)
                        .ok_or_else(|| anyhow!("用法: asa entry remove <地址>"))?;
                    let before = room.entries.len();
                    room.entries.retain(|entry| entry != value);
                    asa::rooms::save(&dirs, &room)?;
                    println!(
                        "已移除 {} 个入口",
                        before.saturating_sub(room.entries.len())
                    );
                }
                "list" | "" => {
                    if room.entries.is_empty() {
                        println!("还没有额外入口。用 asa entry add <地址> 添加。");
                    } else {
                        println!("入口地址:");
                        for entry in &room.entries {
                            println!("  {entry}");
                        }
                    }
                }
                other => bail!("未知 entry 子命令: {other}（可用 add/remove/list）"),
            }
            merge_relays(&mut room, &config);
            let addr = SocketAddr::new(util::lan_ipv4(), opts.port());
            println!();
            println!("长期邀请码（只含入口，不含易变主地址）:");
            println!("  {}", room.invite_long());
            println!();
            println!("完整邀请码:");
            println!("  {}", room.invite(Some(addr)));
            Ok(())
        }
        "relay" => {
            let action = opts.args.first().map(String::as_str).unwrap_or("help");
            match action {
                "serve" => {
                    let port = opts
                        .args
                        .get(1)
                        .and_then(|value| value.parse::<u16>().ok())
                        .unwrap_or(9000);
                    let state = opts.args.get(2).map(PathBuf::from);
                    let bind: SocketAddr = format!("0.0.0.0:{port}")
                        .parse()
                        .context("中继监听地址无效")?;
                    let runtime = tokio::runtime::Builder::new_multi_thread()
                        .enable_all()
                        .thread_name("asa-relay")
                        .build()
                        .context("创建中继运行时失败")?;
                    runtime.block_on(asa::relay::run_relay(bind, state))?;
                    Ok(())
                }
                "add" | "remove" | "rm" | "list" | "" => {
                    let mut room = load_room(&dirs, &opts)?;
                    merge_relays(&mut room, &config);
                    match action {
                        "add" => {
                            let value = opts
                                .args
                                .get(1)
                                .ok_or_else(|| anyhow!("用法: asa relay add <地址>"))?;
                            if !room.relays.iter().any(|item| item == value) {
                                room.relays.push(value.clone());
                            }
                            asa::rooms::save(&dirs, &room)?;
                            if !config.network.relays.iter().any(|item| item == value) {
                                config.network.relays.push(value.clone());
                            }
                            config.save(&asa::config::config_path(&dirs))?;
                            println!("已添加中继: {value}");
                        }
                        "remove" | "rm" => {
                            let value = opts
                                .args
                                .get(1)
                                .ok_or_else(|| anyhow!("用法: asa relay remove <地址>"))?;
                            let before = room.relays.len();
                            room.relays.retain(|item| item != value);
                            asa::rooms::save(&dirs, &room)?;
                            config.network.relays.retain(|item| item != value);
                            config.save(&asa::config::config_path(&dirs))?;
                            println!("已移除 {} 个中继", before.saturating_sub(room.relays.len()));
                        }
                        _ => {
                            if room.relays.is_empty() {
                                println!("还没有中继。用 asa relay add <公网地址:9000> 添加。");
                            } else {
                                println!("中继地址:");
                                for relay in &room.relays {
                                    println!("  {relay}");
                                }
                            }
                        }
                    }
                    let addr = SocketAddr::new(util::lan_ipv4(), opts.port());
                    println!();
                    println!("完整邀请码:");
                    println!("  {}", room.invite(Some(addr)));
                    Ok(())
                }
                other => bail!("未知 relay 子命令: {other}（可用 serve/add/remove/list）"),
            }
        }
        "export" => {
            let path = opts
                .args
                .first()
                .ok_or_else(|| anyhow!("用法: asa export <文件>"))?;
            let mut room = load_room(&dirs, &opts)?;
            merge_relays(&mut room, &config);
            room.save(PathBuf::from(path).as_path())?;
            println!("已导出群文件: {path}");
            println!("注意：该文件包含群密钥，请只通过可信渠道保存和传输。");
            Ok(())
        }
        "import" => {
            let path = opts
                .args
                .first()
                .ok_or_else(|| anyhow!("用法: asa import <文件>"))?;
            let room = RoomConfig::load(PathBuf::from(path).as_path())?
                .ok_or_else(|| anyhow!("群文件不存在或格式不正确: {path}"))?;
            asa::rooms::save(&dirs, &room)?;
            println!("已导入频道 {}（{}）", room.short_id(), room.id);
            println!("入口数量: {}", room.entries.len());
            println!("运行 asa 启动聊天。");
            Ok(())
        }
        "join" => {
            let text = opts
                .invite
                .clone()
                .ok_or_else(|| anyhow!("用法: asa join <邀请码>"))?;
            let (parsed, addresses) = RoomConfig::parse_invite_full(&text)?;
            let room = match asa::rooms::load(&dirs, &parsed.id)? {
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
                    asa::rooms::save(&dirs, &existing)?;
                    existing
                }
                None => {
                    asa::rooms::save(&dirs, &parsed)?;
                    parsed
                }
            };
            let (identity, saved_nick) = Identity::load_or_create(&dirs.identity_path())?;
            let nick = opts
                .name
                .clone()
                .or(saved_nick)
                .unwrap_or_else(|| default_nick(&identity));
            identity.save(&dirs.identity_path(), &nick)?;
            let mut bootstrap = resolve_all(&opts.connect);
            for text in addresses {
                match room::resolve_addr(&text) {
                    Ok(addr) => {
                        if !bootstrap.contains(&addr) {
                            bootstrap.push(addr);
                        }
                    }
                    Err(err) => eprintln!("邀请码中的地址无法解析 {text}: {err}"),
                }
            }
            run_chat(dirs, identity, nick, room, &opts, &config, bootstrap)
        }
        "groups" => {
            let groups = asa::rooms::list(&dirs)?;
            if groups.is_empty() {
                println!("还没有保存的频道。先用 asa init 或 asa join 加入。");
            } else {
                for group in groups {
                    println!(
                        "{} {}  {}",
                        if group.active { "*" } else { " " },
                        group.name,
                        group.id
                    );
                }
            }
            Ok(())
        }
        "chat" => {
            let (identity, saved_nick) = Identity::load_or_create(&dirs.identity_path())?;
            let nick = opts
                .name
                .clone()
                .or(saved_nick)
                .unwrap_or_else(|| default_nick(&identity));
            identity.save(&dirs.identity_path(), &nick)?;
            let room = if let Some(id) = opts.room.as_deref() {
                asa::rooms::load(&dirs, id)?.ok_or_else(|| anyhow!("找不到本地频道: {id}"))?
            } else {
                match tui::lobby(dirs.clone(), identity.clone(), nick.clone())? {
                    Some(room) => room,
                    None => return Ok(()),
                }
            };
            let bootstrap = resolve_all(&opts.connect);
            run_chat(dirs, identity, nick, room, &opts, &config, bootstrap)
        }
        other => bail!("未知命令: {other}\n\n{USAGE}"),
    }
}

fn load_room(dirs: &RuntimeDirs, opts: &Opts) -> Result<RoomConfig> {
    match opts.room.as_deref() {
        Some(id) => asa::rooms::load(dirs, id)?.ok_or_else(|| anyhow!("找不到本地频道: {id}")),
        None => asa::rooms::active(dirs)?
            .ok_or_else(|| anyhow!("还没有频道。先用 asa init 创建，或 asa join 加入")),
    }
}

fn merge_relays(room: &mut RoomConfig, config: &Config) {
    for relay in &config.network.relays {
        if !room.relays.iter().any(|item| item == relay) {
            room.relays.push(relay.clone());
        }
    }
}

fn run_chat(
    dirs: RuntimeDirs,
    identity: Identity,
    nick: String,
    room: RoomConfig,
    opts: &Opts,
    config: &Config,
    bootstrap: Vec<SocketAddr>,
) -> Result<()> {
    let base_nick = nick;
    let mut room = room;
    let mut bootstrap = bootstrap;
    loop {
        let mut profile = GroupProfile::load(&dirs, &room.id).unwrap_or_default();
        let nick = opts
            .name
            .clone()
            .or(profile.nick.clone())
            .unwrap_or_else(|| base_nick.clone());
        if profile.nick.as_deref() != Some(nick.as_str()) {
            profile.nick = Some(nick.clone());
            let _ = profile.save(&dirs, &room.id);
        }
        let downloads = profile.download_dir(config, &dirs);
        let max_file = opts
            .max_file_mb
            .map(|mb| {
                if mb == 0 {
                    u64::MAX
                } else {
                    mb.saturating_mul(1024 * 1024)
                }
            })
            .unwrap_or_else(|| profile.max_file(config));
        let auto_accept = opts.auto_accept || profile.auto_accept(config);
        let mut relays = config.network.relays.clone();
        for relay in &room.relays {
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
        let bind: SocketAddr = format!("0.0.0.0:{}", opts.port())
            .parse()
            .context("监听地址无效")?;
        let mut advertise = Some(SocketAddr::new(util::lan_ipv4(), opts.port()));

        let mut et_status = EtStatus::default();
        let mut et_process = None;
        let mut selected_nodes: Vec<String> = Vec::new();
        let mut extra_entries: Vec<String> = Vec::new();

        let auto_easytier = config.easytier.auto_enable && !config.easytier.nodes.is_empty();
        if profile.easytier_enabled.unwrap_or(auto_easytier) {
            let mut nodes = room.et_nodes.clone();
            if nodes.is_empty() {
                nodes = config.easytier.nodes.clone();
            }
            for node in nodes {
                if let Ok(node) = easytier::normalize_node(&node) {
                    if !selected_nodes.contains(&node) {
                        selected_nodes.push(node);
                    }
                }
            }
            if selected_nodes.is_empty() {
                et_status.error = Some("已开启 EasyTier，但没有可用节点；请先添加节点".to_string());
            } else {
                match EtProcess::start(&dirs, &room, &pubkey, &selected_nodes) {
                    Ok(process) => {
                        advertise = Some(SocketAddr::new(process.ip.into(), opts.port()));
                        et_status.running = true;
                        et_status.ip = Some(process.ip);
                        // 邀请码同时保留局域网入口，ET 不可用时仍可回退。
                        extra_entries.push(format!("{}:{}", util::lan_ipv4(), opts.port()));
                        room.et_nodes = selected_nodes.clone();
                        let _ = asa::rooms::save(&dirs, &room);
                        et_process = Some(process);
                    }
                    Err(err) => {
                        // 回退到局域网/中继，不阻塞进入频道。
                        et_status.error = Some(err.to_string());
                    }
                }
            }
        }
        et_status.nodes = selected_nodes.clone();

        let cfg = NodeConfig {
            signing: identity.signing.clone(),
            pubkey,
            nick: Arc::new(RwLock::new(nick.clone())),
            room_id: room.id.clone(),
            room_key: room.key,
            room_created: room.created_at,
            creator: room.creator.clone(),
            self_since,
            bind,
            advertise,
            extra_entries,
            invite_et_nodes: if et_status.running {
                selected_nodes.clone()
            } else {
                Vec::new()
            },
            dirs: dirs.clone(),
            identity_path: dirs.identity_path(),
            downloads,
            max_file,
            auto_accept,
            mdns: !opts.no_mdns,
            bootstrap: bootstrap.clone(),
            sync_history: config.sync.enabled,
            sync_count: config.sync.count,
            history_limit: config.ui.history_lines,
            relays,
        };

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_name("asa-net")
            .build()
            .context("创建异步运行时失败")?;
        let handle = runtime.block_on(Node::start(cfg))?;
        let channel_name = profile
            .name
            .clone()
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| format!("频道 {}", &room.id[..room.id.len().min(8)]));
        let join_server = if room.creator == handle.pubkey || profile.join_password.is_some() {
            JoinServer::start(
                &room,
                &channel_name,
                &nick,
                handle.advertise.port(),
                &handle.invite,
                profile.join_password.clone(),
            )
            .ok()
        } else {
            None
        };
        let ui = UiConfig {
            room_id: handle.room_id.clone(),
            listen: handle.advertise,
            max_file,
            room_name: profile.name.clone(),
            group_path: asa::config::group_path(&dirs, &room.id),
            groups: asa::rooms::list(&dirs).unwrap_or_default(),
            easytier: et_status,
            config_nodes: config.easytier.nodes.clone(),
            dirs: dirs.clone(),
            vim_mode: config.ui.vim_mode,
            scroll_step: config.ui.scroll_step,
            voice: config.voice.clone(),
        };
        let action = tui::run(handle, ui);
        runtime.shutdown_timeout(Duration::from_secs(2));
        // 切换频道前先停止 EasyTier 和频道目录。
        drop(et_process);
        drop(join_server);
        match action? {
            TuiAction::Quit => break,
            TuiAction::Switch(room_id) => {
                room = asa::rooms::load(&dirs, &room_id)?
                    .ok_or_else(|| anyhow!("找不到本地频道: {room_id}"))?;
                let _ = asa::rooms::set_active(&dirs, &room_id);
                bootstrap.clear();
            }
            TuiAction::Lobby => match tui::lobby(dirs.clone(), identity.clone(), nick)? {
                Some(next) => {
                    room = next;
                    bootstrap.clear();
                }
                None => break,
            },
        }
    }
    Ok(())
}

fn resolve_all(inputs: &[String]) -> Vec<SocketAddr> {
    let mut out = Vec::new();
    for text in inputs {
        for part in text.split(',') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            match room::resolve_addr(part) {
                Ok(addr) => {
                    if !out.contains(&addr) {
                        out.push(addr);
                    }
                }
                Err(err) => eprintln!("忽略无法解析的地址 {part}: {err}"),
            }
        }
    }
    out
}

fn default_nick(identity: &Identity) -> String {
    let user = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| {
            format!(
                "旅行者{}",
                identity.short().chars().take(4).collect::<String>()
            )
        });
    let user: String = user.chars().take(24).collect();
    user
}

fn parse_args(args: Vec<String>) -> Result<Opts> {
    let mut opts = Opts::default();
    let mut iter = args.into_iter().peekable();
    let mut invite_seen = false;

    while let Some(arg) = iter.next() {
        let (flag, inline) = match arg.split_once('=') {
            Some((flag, value)) if flag.starts_with('-') => {
                (flag.to_string(), Some(value.to_string()))
            }
            _ => (arg.clone(), None),
        };
        let mut value = |name: &str| -> Result<String> {
            if let Some(value) = inline.clone() {
                return Ok(value);
            }
            iter.next().ok_or_else(|| anyhow!("选项 {name} 缺少值"))
        };
        match flag.as_str() {
            "-h" | "--help" => {
                opts.command = Some("help".to_string());
                return Ok(opts);
            }
            "-V" | "--version" => {
                opts.command = Some("version".to_string());
                return Ok(opts);
            }
            "--name" | "-n" => opts.name = Some(value("--name")?),
            "--port" | "-p" => {
                let raw = value("--port")?;
                let port: u16 = raw
                    .parse()
                    .with_context(|| format!("端口不是合法数字: {raw}"))?;
                opts.port = Some(port);
            }
            "--connect" | "-c" => opts.connect.push(value("--connect")?),
            "--data-dir" => opts.data_root = Some(PathBuf::from(value("--data-dir")?)),
            "--max-file-mb" => {
                let raw = value("--max-file-mb")?;
                let mb: u64 = raw
                    .parse()
                    .with_context(|| format!("文件上限不是合法数字: {raw}"))?;
                opts.max_file_mb = Some(mb);
            }
            "--no-mdns" => opts.no_mdns = true,
            "--auto-accept" => opts.auto_accept = true,
            "--room" | "-r" => opts.room = Some(value("--room")?),
            other if other.starts_with('-') => bail!("未知选项: {other}\n\n{USAGE}"),
            positional => {
                if opts.command.is_none() {
                    opts.command = Some(positional.to_string());
                } else if !invite_seen && opts.command.as_deref() == Some("join") {
                    opts.invite = Some(positional.to_string());
                    invite_seen = true;
                } else {
                    opts.args.push(positional.to_string());
                }
            }
        }
    }
    Ok(opts)
}

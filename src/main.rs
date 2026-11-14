//! ASA 命令行入口。

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};

use asa::config::{Config, GroupProfile};
use asa::node::{Node, NodeConfig};
use asa::paths::{self, RuntimeDirs};
use asa::room::{self, RoomConfig};
use asa::settings::Settings;
use asa::tui::{self, UiConfig};
use asa::{identity::Identity, util};

const USAGE: &str = "\
ASA · 去中心化 TUI 群聊

用法:
  asa init [--name 昵称] [--port 7777]     创建本地身份与群，打印邀请码
  asa join <邀请码> [--name 昵称]          用邀请码加入群
  asa chat [选项]                          启动聊天（默认命令）
  asa invite [--port 7777]                 打印当前群的邀请码
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
            let mut room = match RoomConfig::load(&dirs.room_path())? {
                Some(room) => room,
                None => {
                    let room = RoomConfig::create(&identity);
                    room.save(&dirs.room_path())?;
                    room
                }
            };
            merge_relays(&mut room, &config);
            room.save(&dirs.room_path())?;
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
            let mut room = RoomConfig::load(&dirs.room_path())?
                .ok_or_else(|| anyhow!("还没有群，请先运行 asa init"))?;
            merge_relays(&mut room, &config);
            let addr = SocketAddr::new(util::lan_ipv4(), opts.port());
            println!("{}", room.invite(Some(addr)));
            Ok(())
        }
        "entry" => {
            let mut room = RoomConfig::load(&dirs.room_path())?
                .ok_or_else(|| anyhow!("还没有群，请先运行 asa init 或 asa join"))?;
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
                    room.save(&dirs.room_path())?;
                    println!("已添加入口: {value}");
                }
                "remove" | "rm" => {
                    let value = opts
                        .args
                        .get(1)
                        .ok_or_else(|| anyhow!("用法: asa entry remove <地址>"))?;
                    let before = room.entries.len();
                    room.entries.retain(|entry| entry != value);
                    room.save(&dirs.room_path())?;
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
                    let state = opts
                        .args
                        .get(2)
                        .map(PathBuf::from)
                        .unwrap_or_else(|| dirs.root.join("relay-state.json"));
                    let bind: SocketAddr = format!("0.0.0.0:{port}")
                        .parse()
                        .context("中继监听地址无效")?;
                    let runtime = tokio::runtime::Builder::new_multi_thread()
                        .enable_all()
                        .thread_name("asa-relay")
                        .build()
                        .context("创建中继运行时失败")?;
                    runtime.block_on(asa::relay::run_relay(bind, Some(state)))?;
                    Ok(())
                }
                "add" | "remove" | "rm" | "list" | "" => {
                    let mut room = RoomConfig::load(&dirs.room_path())?
                        .ok_or_else(|| anyhow!("还没有群，请先运行 asa init 或 asa join"))?;
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
                            room.save(&dirs.room_path())?;
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
                            room.save(&dirs.room_path())?;
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
            let mut room = RoomConfig::load(&dirs.room_path())?
                .ok_or_else(|| anyhow!("还没有群，请先运行 asa init 或 asa join"))?;
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
            room.save(&dirs.room_path())?;
            println!("已导入群 {}（{}）", room.short_id(), room.id);
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
            let room = match RoomConfig::load(&dirs.room_path())? {
                Some(mut existing) if existing.id == parsed.id => {
                    for entry in &parsed.entries {
                        if !existing.entries.iter().any(|e| e == entry) {
                            existing.entries.push(entry.clone());
                        }
                    }
                    existing.save(&dirs.room_path())?;
                    existing
                }
                _ => {
                    parsed.save(&dirs.room_path())?;
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
        "chat" => {
            let room = RoomConfig::load(&dirs.room_path())?
                .ok_or_else(|| anyhow!("还没有群。请先运行 asa init 或 asa join <邀请码>"))?;
            let (identity, saved_nick) = Identity::load_or_create(&dirs.identity_path())?;
            let nick = opts
                .name
                .clone()
                .or(saved_nick)
                .unwrap_or_else(|| default_nick(&identity));
            identity.save(&dirs.identity_path(), &nick)?;
            let bootstrap = resolve_all(&opts.connect);
            run_chat(dirs, identity, nick, room, &opts, &config, bootstrap)
        }
        other => bail!("未知命令: {other}\n\n{USAGE}"),
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
    let mut profile = GroupProfile::load(&dirs, &room.id).unwrap_or_default();
    let nick = opts.name.clone().or(profile.nick.clone()).unwrap_or(nick);
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
    let advertise_ip = util::lan_ipv4();
    let advertise = Some(SocketAddr::new(advertise_ip, opts.port()));

    let cfg = NodeConfig {
        signing: identity.signing.clone(),
        pubkey,
        nick: Arc::new(RwLock::new(nick)),
        room_id: room.id.clone(),
        room_key: room.key,
        room_created: room.created_at,
        creator: room.creator.clone(),
        self_since,
        bind,
        advertise,
        dirs: dirs.clone(),
        identity_path: dirs.identity_path(),
        downloads,
        max_file,
        auto_accept,
        mdns: !opts.no_mdns,
        bootstrap,
        relays,
    };

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("asa-net")
        .build()
        .context("创建异步运行时失败")?;
    let handle = runtime.block_on(Node::start(cfg))?;
    let ui = UiConfig {
        room_id: handle.room_id.clone(),
        listen: handle.advertise,
        max_file,
        room_name: profile.name.clone(),
        group_path: asa::config::group_path(&dirs, &room.id),
    };
    let result = tui::run(handle, ui);
    runtime.shutdown_timeout(Duration::from_secs(2));
    result
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

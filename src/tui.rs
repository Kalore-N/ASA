//! TUI 界面：聊天区、成员区、传输区、输入行。

use std::io::Write as _;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::Child;
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::cursor;
use crossterm::event::{
    self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers,
};
use crossterm::terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph, Scrollbar,
    ScrollbarOrientation, ScrollbarState,
};
use ratatui::{Frame, Terminal};
use tokio::sync::mpsc::UnboundedSender;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::lan::{ChannelBrowser, DiscoveredChannel};
use crate::node::{Cmd, MemberView, NodeEvent, NodeHandle};
use crate::proto::ReplyRef;
use crate::util;

const HELP: &str = "\
命令列表：
/help                 显示这份帮助
/name <昵称>          修改昵称（会通知频道）
/rename <名称>        修改本机显示的频道名称（仅本地）
/invite               显示邀请码，发给朋友加入频道
/peers                查看在线成员
/connect <ip:port>    手动连接一个成员（跨网时用）
/send <成员1,成员2|@all> <路径|目录|通配符>
                      发送文件/文件夹（递归）；支持 *.txt、dir/*；/send 空参多选成员
/accept <编号>        接受收到的文件
/deny <编号>          拒绝收到的文件
/files                查看进行中的传输
/sync                 向在线成员请求最近聊天记录
/channels             打开频道选择框
/switch <名称|ID>     切换到指定频道
/et [status|on|off|add <地址>|list|remove <编号>]
                      EasyTier 节点与当前频道内网穿透
/et use <编号> | test [编号]
                      选择本频道使用的节点、测试节点延迟
/call <昵称> | accept|reject|hangup|mute|ptt
                      1:1 语音通话
/voice join|leave|mute|members
                      频道常驻语音房（多人同时通话）
/voice <昵称> | stop | play <路径>
                      录制/发送/播放语音消息
/reply [编号]         回复指定消息；/reply 列出可回复的编号
/copy                 把最近的邀请码或保存路径复制到系统剪贴板（OSC 52）
/mouse [on|off]       切换鼠标捕获；关闭后可用鼠标拖选文本（也可按住 Shift 拖选）
Ctrl+Y                把最近内容放入输入行（剪贴板不可用时的兜底）
/clear                清空当前屏幕（不影响本地日志）
/quit                 退出
回复：Ctrl+R 进入选择，↑/↓ 选择消息，Enter 确认，Esc 取消。
提及：输入 @昵称 或 @所有人；输入 @ 后按 Tab 从在线成员中补全。
补全：输入 / 后按 Tab 补全指令和二级参数（/et、/call、/voice、/switch）；
      /send 与 /voice play 支持文件/文件夹路径补全，@ 支持成员补全。
语音房：浏览模式按 v 加入/退出，m 静音；语音房与 1:1 通话互斥。
提示：直接输入文字回车即发送；PgUp/PgDn 翻看历史，End 回到底部。";

/// 本次进程是否已经尝试过 EasyTier 提权，避免自动提权失败后反复重启循环。
static ELEVATE_ATTEMPTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

const HINTS: &str =
    "Enter 发送 · Tab 补全 · Ctrl+R 回复 · PgUp/PgDn 翻页 · /help 命令 · Ctrl+C 退出";

/// 可补全的聊天指令。
const COMMANDS: [&str; 21] = [
    "/help",
    "/name",
    "/rename",
    "/invite",
    "/peers",
    "/connect",
    "/send",
    "/accept",
    "/deny",
    "/files",
    "/sync",
    "/channels",
    "/switch",
    "/et",
    "/call",
    "/voice",
    "/copy",
    "/mouse",
    "/clear",
    "/reply",
    "/quit",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TuiAction {
    Quit,
    /// 切换到指定频道（重启节点）。
    Switch(String),
    /// 返回频道选择大厅。
    Lobby,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Chat,
    System,
}

struct Message {
    seq: u64,
    ts: u64,
    nick: String,
    text: String,
    kind: Kind,
    mine: bool,
    id: String,
    mentions: Vec<String>,
    reply: Option<ReplyRef>,
}

#[derive(Clone)]
struct Completion {
    /// 替换起始位置（字符索引）。
    start: usize,
    /// 上次插入后的结束位置（字符索引），含尾随空格。
    end: usize,
    candidates: Vec<String>,
    index: usize,
}

struct FileSelect {
    members: Vec<MemberView>,
    selected: Vec<bool>,
    index: usize,
}

struct Offer {
    xfer: String,
    from_nick: String,
    name: String,
    size: u64,
}

struct Transfer {
    xfer: String,
    name: String,
    done: u64,
    total: u64,
    outgoing: bool,
}

pub struct UiConfig {
    pub room_id: String,
    pub listen: SocketAddr,
    pub max_file: u64,
    pub room_name: Option<String>,
    pub group_path: PathBuf,
    pub groups: Vec<crate::rooms::SavedGroup>,
    pub easytier: crate::easytier::EtStatus,
    pub config_nodes: Vec<String>,
    pub dirs: crate::paths::RuntimeDirs,
    pub vim_mode: bool,
    pub scroll_step: u16,
    pub voice: crate::config::VoicePrefs,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum InputMode {
    Normal,
    Insert,
}

pub struct App {
    cmd: UnboundedSender<Cmd>,
    input: String,
    cursor: usize,
    messages: Vec<Message>,
    members: Vec<MemberView>,
    host: Option<String>,
    me: String,
    nick: String,
    peer_id: String,
    room_id: String,
    room_name: Option<String>,
    group_path: PathBuf,
    listen: SocketAddr,
    invite: String,
    max_file: u64,
    transfers: Vec<Transfer>,
    offers: Vec<Offer>,
    status: Option<(String, Instant, bool)>,
    scroll: usize,
    follow: bool,
    history: Vec<String>,
    hist_pos: Option<usize>,
    should_quit: bool,
    wide_layout: bool,
    next_seq: u64,
    /// 待发送的回复：#编号 + 引用内容。
    pending_reply: Option<(u64, ReplyRef)>,
    /// 回复选择模式下当前选中的消息索引。
    reply_select: Option<usize>,
    /// 当前 Tab 补全状态。
    completion: Option<Completion>,
    /// 多选文件接收人模式。
    file_select: Option<FileSelect>,
    /// 最近一次可复制内容（邀请码或保存路径）。
    last_copy: Option<String>,
    groups: Vec<crate::rooms::SavedGroup>,
    action: Option<TuiAction>,
    easytier: crate::easytier::EtStatus,
    config_nodes: Vec<String>,
    dirs: crate::paths::RuntimeDirs,
    input_mode: InputMode,
    vim_mode: bool,
    scroll_step: u16,
    voice: crate::config::VoicePrefs,
    active_call: Option<ActiveCall>,
    incoming_call: Option<IncomingCall>,
    voice_message: Option<VoiceMessage>,
    ptt_active: bool,
    /// 频道常驻语音房。
    voice_room_joined: bool,
    voice_room_members: Vec<crate::proto::VoiceRoomMember>,
    voice_room_audio: Option<crate::audio::VoiceAudio>,
    voice_room_muted: bool,
    /// 上次向上游发送语音帧的时间，用于限速（避免突发采集把节点事件队列饿死）。
    voice_last_send: Instant,
    /// 是否捕获鼠标；关闭后终端可拖选文本、使用系统右键菜单。
    mouse_capture: bool,
    /// EasyTier 缺少 TUN 权限时等待用户确认的核心路径。
    elevate_core: Option<PathBuf>,
}

struct ActiveCall {
    nick: String,
    audio: Option<crate::audio::VoiceAudio>,
    muted: bool,
}

struct IncomingCall {
    nick: String,
    call_id: String,
}

struct VoiceMessage {
    child: Child,
    path: PathBuf,
    target: String,
}

impl App {
    fn new(handle: &NodeHandle, cfg: UiConfig) -> Self {
        let mut app = App {
            cmd: handle.cmd.clone(),
            input: String::new(),
            cursor: 0,
            messages: Vec::new(),
            members: Vec::new(),
            host: None,
            me: handle.pubkey.clone(),
            nick: handle.peer_id.clone(),
            peer_id: handle.peer_id.clone(),
            room_id: cfg.room_id,
            room_name: cfg.room_name,
            group_path: cfg.group_path,
            listen: cfg.listen,
            invite: handle.invite.clone(),
            max_file: cfg.max_file,
            transfers: Vec::new(),
            offers: Vec::new(),
            status: None,
            scroll: 0,
            follow: true,
            history: Vec::new(),
            hist_pos: None,
            should_quit: false,
            wide_layout: true,
            next_seq: 1,
            pending_reply: None,
            reply_select: None,
            completion: None,
            file_select: None,
            last_copy: None,
            groups: cfg.groups,
            action: None,
            easytier: cfg.easytier,
            config_nodes: cfg.config_nodes,
            dirs: cfg.dirs,
            input_mode: if cfg.vim_mode {
                InputMode::Normal
            } else {
                InputMode::Insert
            },
            vim_mode: cfg.vim_mode,
            scroll_step: cfg.scroll_step.max(1),
            voice: cfg.voice,
            active_call: None,
            incoming_call: None,
            voice_message: None,
            ptt_active: false,
            voice_room_joined: false,
            voice_room_members: Vec::new(),
            voice_room_audio: None,
            voice_room_muted: false,
            voice_last_send: Instant::now(),
            mouse_capture: true,
            elevate_core: None,
        };
        app.push_system("欢迎使用 ASA。直接输入文字回车发送；/help 查看命令；/invite 获取邀请码。");
        if let Some(err) = app.easytier.error.clone() {
            app.push_system(format!("EasyTier 未启动：{err}"));
            if err.contains("Operation not permitted")
                || err.contains("无法创建 TUN")
                || err.contains("TUN 权限")
            {
                let config = crate::config::Config::ensure(&app.dirs).unwrap_or_default();
                match crate::easytier::find_core() {
                    Ok(core)
                        if config.easytier.auto_elevate
                            && !ELEVATE_ATTEMPTED.load(std::sync::atomic::Ordering::Relaxed) =>
                    {
                        app.set_status("EasyTier 需要 TUN 权限，正在打开提权窗口…", false);
                        app.run_elevation(true, core);
                    }
                    Ok(core)
                        if !config.easytier.elevate_prompted
                            && !ELEVATE_ATTEMPTED.load(std::sync::atomic::Ordering::Relaxed) =>
                    {
                        app.elevate_core = Some(core);
                        app.push_system(
                            "EasyTier 缺少 TUN 权限：y=提权并记住 · o=仅本次 · n=不再提示\n\
                             选择会写入 config.json 的 easytier.auto_elevate；\
                             把 elevate_prompted 改为 false 可再次看到提示。"
                                .to_string(),
                        );
                    }
                    _ => {}
                }
            }
        }
        app
    }

    // ------------------------------------------------------------ 事件

    fn on_event(&mut self, event: NodeEvent) {
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
            } => {
                let _ = from;
                let mentioned = !mine && mentions.iter().any(|pk| pk == &self.me);
                if mentioned {
                    self.set_status(format!("{nick} 提到了你"), false);
                }
                let message = Message {
                    seq: 0,
                    ts,
                    nick,
                    text,
                    kind: Kind::Chat,
                    mine,
                    id,
                    mentions,
                    reply,
                };
                if history {
                    self.insert_message_sorted(message);
                } else {
                    self.push_message(message);
                }
            }
            NodeEvent::System { text, ts } => {
                self.push_message(Message {
                    seq: 0,
                    ts,
                    nick: "系统".to_string(),
                    text,
                    kind: Kind::System,
                    mine: false,
                    id: String::new(),
                    mentions: Vec::new(),
                    reply: None,
                });
            }
            NodeEvent::Members { members, host, me } => {
                if let Some(view) = members.iter().find(|m| m.pubkey == me) {
                    self.nick = view.nick.clone();
                    self.peer_id = view.peer_id.clone();
                }
                self.host = host;
                self.members = members;
                self.me = me;
            }
            NodeEvent::FileOffer {
                xfer,
                from_nick,
                name,
                size,
                ..
            } => {
                self.offers.push(Offer {
                    xfer,
                    from_nick,
                    name,
                    size,
                });
            }
            NodeEvent::FileProgress {
                xfer,
                name,
                done,
                total,
                outgoing,
            } => {
                if let Some(item) = self.transfers.iter_mut().find(|t| t.xfer == xfer) {
                    item.done = done;
                    item.total = total;
                } else {
                    self.transfers.push(Transfer {
                        xfer,
                        name,
                        done,
                        total,
                        outgoing,
                    });
                }
            }
            NodeEvent::FileComplete {
                xfer,
                ok,
                msg,
                outgoing,
                path,
                name,
            } => {
                self.transfers.retain(|t| t.xfer != xfer);
                self.offers.retain(|o| o.xfer != xfer);
                if !ok {
                    self.set_status(format!("文件传输失败: {name} · {msg}"), true);
                } else if outgoing {
                    self.set_status(format!("已发送 {name}"), false);
                } else {
                    let where_ = path
                        .as_ref()
                        .map(|p| p.display().to_string())
                        .unwrap_or_else(|| msg.clone());
                    if let Some(path) = &path {
                        self.last_copy = Some(path.display().to_string());
                    } else {
                        self.last_copy = Some(where_.clone());
                    }
                    self.set_status(
                        format!("已接收 {name} → {where_}（Ctrl+Y 放入输入行）"),
                        false,
                    );
                }
            }
            NodeEvent::RoomName(name) => {
                self.room_name = Some(name);
            }
            NodeEvent::CallRinging { nick, .. } => {
                self.set_status(format!("正在呼叫 {nick}…"), false);
            }
            NodeEvent::IncomingCall {
                peer,
                nick,
                call_id,
            } => {
                let _ = peer;
                self.incoming_call = Some(IncomingCall {
                    nick: nick.clone(),
                    call_id: call_id.clone(),
                });
                self.set_status(
                    format!("{nick} 来电：/call accept 接听，/call reject 拒绝"),
                    false,
                );
            }
            NodeEvent::CallAccepted {
                peer,
                nick,
                call_id,
            } => {
                self.incoming_call = None;
                if self.voice_room_joined {
                    let _ = self.cmd.send(Cmd::VoiceRoomLeave);
                    self.voice_room_joined = false;
                    self.voice_room_audio = None;
                }
                let audio = if self.voice.enabled {
                    match crate::audio::check_tools().and_then(|_| crate::audio::start(&self.voice))
                    {
                        Ok(audio) => Some(audio),
                        Err(err) => {
                            self.set_status(format!("音频启动失败: {err}"), true);
                            self.push_system(format!(
                                "语音通话未能开始：{err}\n\
                                 请检查系统声音服务/麦克风；仍失败时在配置中设置 \
                                 voice.input_device 与 voice.output_device（例如 default 或 plughw:0,0）。"
                            ));
                            let _ = self.cmd.send(Cmd::HangupCall);
                            None
                        }
                    }
                } else {
                    self.set_status("语音功能未启用", true);
                    let _ = self.cmd.send(Cmd::HangupCall);
                    None
                };
                let _ = (peer, call_id);
                self.active_call = Some(ActiveCall {
                    nick: nick.clone(),
                    audio,
                    muted: false,
                });
                let device = self
                    .active_call
                    .as_ref()
                    .and_then(|call| call.audio.as_ref())
                    .map(|audio| {
                        format!(
                            " · 输入 {} · 输出 {}",
                            audio.input_device, audio.output_device
                        )
                    })
                    .unwrap_or_default();
                self.set_status(format!("与 {nick} 的通话已接通{device}"), false);
            }
            NodeEvent::CallEnded { nick, reason, .. } => {
                self.active_call = None;
                self.incoming_call = None;
                self.set_status(format!("与 {nick} 的通话结束: {reason}"), false);
            }
            NodeEvent::AudioFrame { data } => {
                if let Some(call) = self.active_call.as_ref() {
                    if let Some(audio) = call.audio.as_ref() {
                        audio.player.play(data);
                    }
                }
            }
            NodeEvent::VoiceRoomState { joined, members } => {
                self.voice_room_members = members;
                if joined && !self.voice_room_joined {
                    self.voice_room_joined = true;
                    if let Err(err) = self.start_voice_room_audio() {
                        let _ = self.cmd.send(Cmd::VoiceRoomLeave);
                        self.voice_room_joined = false;
                        self.voice_room_audio = None;
                        self.set_status(format!("语音房音频启动失败: {err}"), true);
                        self.push_system(format!(
                            "语音房未能开始：{err}\n请检查系统声音服务/麦克风或配置 voice.input_device。"
                        ));
                    }
                } else if !joined && self.voice_room_joined {
                    self.voice_room_joined = false;
                    self.voice_room_audio = None;
                }
            }
            NodeEvent::VoiceRoomAudio { data, .. } => {
                if self.voice_room_joined {
                    if let Some(audio) = self.voice_room_audio.as_ref() {
                        audio.player.play(data);
                    }
                }
            }
            NodeEvent::Info(text) => {
                let error = text.contains("失败") || text.contains("错误") || text.contains("无法");
                self.set_status(text, error);
            }
            NodeEvent::Shutdown => {
                self.should_quit = true;
            }
        }
    }

    fn push_message(&mut self, mut message: Message) {
        message.seq = self.next_seq;
        self.next_seq += 1;
        self.messages.push(message);
        if self.messages.len() > 5000 {
            self.messages.drain(..1000);
        }
        if self.follow {
            self.scroll = 0;
        }
    }

    fn insert_message_sorted(&mut self, mut message: Message) {
        message.seq = self.next_seq;
        self.next_seq += 1;
        let position = self
            .messages
            .iter()
            .position(|existing| existing.ts > message.ts)
            .unwrap_or(self.messages.len());
        self.messages.insert(position, message);
        if self.messages.len() > 5000 {
            self.messages.drain(..1000);
        }
        if self.follow {
            self.scroll = 0;
        }
    }

    fn push_system(&mut self, text: impl Into<String>) {
        self.push_message(Message {
            seq: 0,
            ts: util::now_ms(),
            nick: "系统".to_string(),
            text: text.into(),
            kind: Kind::System,
            mine: false,
            id: String::new(),
            mentions: Vec::new(),
            reply: None,
        });
    }

    fn set_status(&mut self, text: impl Into<String>, error: bool) {
        self.status = Some((text.into(), Instant::now(), error));
    }

    // ------------------------------------------------------------ 键盘

    fn on_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if let Some(core) = self.elevate_core.clone() {
            match key.code {
                KeyCode::Char('y') => self.run_elevation(true, core),
                KeyCode::Char('o') => self.run_elevation(false, core),
                KeyCode::Char('n') | KeyCode::Esc => {
                    self.elevate_core = None;
                    self.save_elevate_preference(false);
                    self.set_status(
                        "已跳过提权；可手动运行 sudo setcap cap_net_admin,cap_net_raw+ep <easytier-core>",
                        true,
                    );
                }
                _ => {}
            }
            return;
        }
        if self.file_select.is_some() {
            self.on_file_select_key(key);
            return;
        }
        if self.reply_select.is_some() {
            match key.code {
                KeyCode::Up => self.move_reply_select(-1),
                KeyCode::Down => self.move_reply_select(1),
                KeyCode::Enter => self.confirm_reply_select(),
                KeyCode::Esc => self.reply_select = None,
                KeyCode::Char('c') if ctrl => self.should_quit = true,
                KeyCode::Char('r') if ctrl => self.reply_select = None,
                _ => {}
            }
            return;
        }
        if self.vim_mode {
            if self.input_mode == InputMode::Normal {
                if self.on_normal_key(key) {
                    return;
                }
            } else if key.code == KeyCode::Esc && !ctrl {
                self.input_mode = InputMode::Normal;
                self.completion = None;
                return;
            }
        }
        match key.code {
            KeyCode::Char('c') if ctrl => self.should_quit = true,
            KeyCode::Char('l') if ctrl => {
                self.messages.clear();
                self.reply_select = None;
                self.pending_reply = None;
            }
            KeyCode::Char('u') if ctrl => {
                self.input.clear();
                self.cursor = 0;
                self.completion = None;
            }
            KeyCode::Char('r') if ctrl => self.start_reply_select(),
            KeyCode::Char('y') if ctrl => self.copy_to_input(),
            KeyCode::Enter => self.submit(),
            KeyCode::Backspace => {
                self.completion = None;
                self.backspace();
            }
            KeyCode::Delete => {
                self.completion = None;
                self.delete_forward();
            }
            KeyCode::Left => {
                self.completion = None;
                if self.cursor > 0 {
                    self.cursor -= 1;
                }
            }
            KeyCode::Right => {
                self.completion = None;
                if self.cursor < self.input.chars().count() {
                    self.cursor += 1;
                }
            }
            KeyCode::Home => {
                self.completion = None;
                self.cursor = 0;
            }
            KeyCode::End => {
                self.completion = None;
                self.cursor = self.input.chars().count();
                self.scroll = 0;
                self.follow = true;
            }
            KeyCode::Up => {
                self.completion = None;
                self.history_prev();
            }
            KeyCode::Down => {
                self.completion = None;
                self.history_next();
            }
            KeyCode::PageUp => self.scroll_up(10),
            KeyCode::PageDown => self.scroll_down(10),
            KeyCode::Tab => self.complete(false),
            KeyCode::BackTab => self.complete(true),
            KeyCode::Esc => {
                if self.pending_reply.take().is_some() {
                    self.set_status("已取消回复", false);
                }
                self.input.clear();
                self.cursor = 0;
                self.completion = None;
            }
            KeyCode::Char(ch) if !ctrl && !key.modifiers.contains(KeyModifiers::ALT) => {
                self.completion = None;
                self.insert_text(&ch.to_string());
            }
            _ => {}
        }
    }

    /// 浏览模式按键；返回 true 表示已处理。
    fn on_normal_key(&mut self, key: KeyEvent) -> bool {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl {
            // Ctrl+C/L/U/R/Y 交给主键盘处理。
            return !matches!(key.code, KeyCode::Char('c' | 'l' | 'u' | 'r' | 'y'));
        }
        if self.incoming_call.is_some() {
            match key.code {
                KeyCode::Char('y') => {
                    if let Some(incoming) = self.incoming_call.take() {
                        let _ = self.cmd.send(Cmd::AcceptCall {
                            call_id: incoming.call_id,
                        });
                        self.set_status(format!("接听 {}…", incoming.nick), false);
                    }
                    return true;
                }
                KeyCode::Char('n') => {
                    if let Some(incoming) = self.incoming_call.take() {
                        let _ = self.cmd.send(Cmd::RejectCall {
                            call_id: incoming.call_id,
                        });
                        self.set_status("已拒绝来电", false);
                    }
                    return true;
                }
                _ => {}
            }
        }
        if self.active_call.is_some() {
            match key.code {
                KeyCode::Char('m') => {
                    let muted = if let Some(call) = self.active_call.as_mut() {
                        call.muted = !call.muted;
                        call.muted
                    } else {
                        false
                    };
                    self.set_status(
                        if muted {
                            "通话已静音"
                        } else {
                            "通话已取消静音"
                        },
                        false,
                    );
                    return true;
                }
                KeyCode::Char('h') => {
                    let _ = self.cmd.send(Cmd::HangupCall);
                    self.set_status("正在挂断…", false);
                    return true;
                }
                KeyCode::Char('p') if self.voice.push_to_talk => {
                    self.ptt_active = !self.ptt_active;
                    self.set_status(
                        if self.ptt_active {
                            "按住说话：开"
                        } else {
                            "按住说话：关"
                        },
                        false,
                    );
                    return true;
                }
                _ => {}
            }
        }
        match key.code {
            KeyCode::Char('i') | KeyCode::Char('a') | KeyCode::Enter => {
                self.input_mode = InputMode::Insert;
                self.completion = None;
            }
            KeyCode::Char('/') => {
                self.input_mode = InputMode::Insert;
                self.insert_text("/");
            }
            KeyCode::Char('v') => self.toggle_voice_room(),
            KeyCode::Char('j') | KeyCode::Down => self.scroll_down(self.scroll_step as usize),
            KeyCode::Char('k') | KeyCode::Up => self.scroll_up(self.scroll_step as usize),
            KeyCode::Char('g') => self.scroll_top(),
            KeyCode::Char('G') => self.scroll_bottom(),
            KeyCode::PageDown => self.scroll_down(10),
            KeyCode::PageUp => self.scroll_up(10),
            KeyCode::Home => self.scroll_top(),
            KeyCode::End => self.scroll_bottom(),
            KeyCode::Esc => {}
            _ => {}
        }
        true
    }

    fn scroll_up(&mut self, lines: usize) {
        self.scroll = self.scroll.saturating_add(lines);
        self.follow = false;
    }

    fn scroll_down(&mut self, lines: usize) {
        self.scroll = self.scroll.saturating_sub(lines);
        if self.scroll == 0 {
            self.follow = true;
        }
    }

    fn scroll_top(&mut self) {
        self.scroll = 100_000;
        self.follow = false;
    }

    fn scroll_bottom(&mut self) {
        self.scroll = 0;
        self.follow = true;
    }

    /// 语音帧发送限速：每帧间隔最多发一帧，丢弃积压的更旧帧以降低延迟。
    fn voice_frame_due(&mut self) -> bool {
        let interval = Duration::from_millis(self.voice.frame_ms.clamp(10, 60) as u64);
        if self.voice_last_send.elapsed() < interval {
            return false;
        }
        self.voice_last_send = Instant::now();
        true
    }

    fn pump_voice(&mut self) {
        let push_to_talk = self.voice.push_to_talk;
        let ptt = self.ptt_active;
        let mut latest = None;
        let mut failed: Option<String> = None;
        if let Some(call) = self.active_call.as_mut() {
            if let Some(audio) = call.audio.as_mut() {
                while let Some(frame) = audio.capture.try_recv() {
                    if !call.muted && (!push_to_talk || ptt) {
                        latest = Some(frame);
                    }
                }
                failed = audio.capture.failure().or_else(|| audio.player.failure());
            }
        }
        if let Some(frame) = latest {
            if self.voice_frame_due() {
                let _ = self.cmd.send(Cmd::AudioFrame { data: frame });
            }
        }
        if let Some(err) = failed {
            let nick = self
                .active_call
                .as_ref()
                .map(|call| call.nick.clone())
                .unwrap_or_default();
            self.active_call = None;
            let _ = self.cmd.send(Cmd::HangupCall);
            self.set_status(format!("与 {nick} 的通话中断: {err}"), true);
        }
    }

    fn start_voice_room_audio(&mut self) -> anyhow::Result<()> {
        if !self.voice.enabled {
            anyhow::bail!("语音功能未启用");
        }
        if self.active_call.is_some() {
            anyhow::bail!("通话中不能加入语音房");
        }
        if self.voice_message.is_some() {
            anyhow::bail!("录音中不能加入语音房");
        }
        crate::audio::check_tools()?;
        let audio = crate::audio::start(&self.voice)?;
        self.push_system(format!(
            "语音房音频：输入 {} · 输出 {}",
            audio.input_device, audio.output_device
        ));
        self.voice_room_audio = Some(audio);
        Ok(())
    }

    fn save_elevate_preference(&mut self, auto: bool) {
        if let Ok(mut config) = crate::config::Config::ensure(&self.dirs) {
            config.easytier.auto_elevate = auto;
            config.easytier.elevate_prompted = true;
            let _ = config.save(&crate::config::config_path(&self.dirs));
        }
    }

    fn run_elevation(&mut self, remember: bool, core: PathBuf) {
        self.elevate_core = None;
        ELEVATE_ATTEMPTED.store(true, std::sync::atomic::Ordering::Relaxed);
        self.save_elevate_preference(remember);
        // 先尝试 pkexec 的图形授权窗口；setcap 是一次性授予，不需要长期以 root 运行。
        let setcap = ["/usr/bin/setcap", "/usr/sbin/setcap", "/sbin/setcap"]
            .iter()
            .map(PathBuf::from)
            .find(|path| path.exists())
            .unwrap_or_else(|| PathBuf::from("setcap"));
        let status = std::process::Command::new("pkexec")
            .arg(&setcap)
            .args(["cap_net_admin,cap_net_raw+ep"])
            .arg(&core)
            .status();
        match status {
            Ok(status) if status.success() => {
                self.push_system(format!(
                    "已为 {} 添加 TUN 权限，正在重试 EasyTier…",
                    core.display()
                ));
                self.restart_for_et();
            }
            Ok(status) => {
                self.set_status(format!("提权未完成（{status}）"), true);
                self.push_system(format!(
                    "提权未完成。可手动执行：\nsudo setcap cap_net_admin,cap_net_raw+ep {}",
                    core.display()
                ));
            }
            Err(err) => {
                self.set_status(format!("无法启动 pkexec: {err}"), true);
                self.push_system(format!(
                    "无法打开提权窗口（{err}）。请手动执行：\nsudo setcap cap_net_admin,cap_net_raw+ep {}",
                    core.display()
                ));
            }
        }
    }

    fn toggle_voice_room(&mut self) {
        if self.voice_room_joined {
            let _ = self.cmd.send(Cmd::VoiceRoomLeave);
            self.voice_room_joined = false;
            self.voice_room_audio = None;
            self.set_status("正在退出语音房…", false);
            return;
        }
        if self.active_call.is_some() {
            self.set_status("通话中不能加入语音房，请先挂断", true);
            return;
        }
        if self.voice_message.is_some() {
            self.set_status("录音中不能加入语音房，请先 /voice stop", true);
            return;
        }
        let _ = self.cmd.send(Cmd::VoiceRoomJoin);
        self.set_status("正在加入语音房…", false);
    }

    fn pump_voice_room(&mut self) {
        let muted = self.voice_room_muted;
        let mut latest = None;
        let mut failed: Option<String> = None;
        if let Some(audio) = self.voice_room_audio.as_mut() {
            while let Some(frame) = audio.capture.try_recv() {
                if !muted {
                    latest = Some(frame);
                }
            }
            failed = audio.capture.failure().or_else(|| audio.player.failure());
        }
        if let Some(frame) = latest {
            if self.voice_frame_due() {
                let _ = self.cmd.send(Cmd::VoiceRoomFrame { data: frame });
            }
        }
        if let Some(err) = failed {
            self.voice_room_audio = None;
            self.voice_room_joined = false;
            let _ = self.cmd.send(Cmd::VoiceRoomLeave);
            self.set_status(format!("语音房音频中断: {err}"), true);
        }
    }

    fn on_mouse(&mut self, mouse: crossterm::event::MouseEvent) {
        match mouse.kind {
            crossterm::event::MouseEventKind::ScrollUp => self.scroll_up(self.scroll_step as usize),
            crossterm::event::MouseEventKind::ScrollDown => {
                self.scroll_down(self.scroll_step as usize)
            }
            _ => {}
        }
    }

    // ------------------------------------------------------------ 文件接收人多选

    fn resolve_send_targets(&self, text: &str) -> Vec<String> {
        let text = text.trim();
        if text == "@all" || text == "@所有人" || text.eq_ignore_ascii_case("all") {
            return self
                .members
                .iter()
                .filter(|member| !member.me && member.pubkey != self.me)
                .map(|member| member.pubkey.clone())
                .collect();
        }
        text.split(',')
            .map(|item| item.trim().trim_start_matches('@'))
            .filter(|item| !item.is_empty())
            .map(|item| item.to_string())
            .collect()
    }

    fn start_file_select(&mut self) {
        let members: Vec<MemberView> = self
            .members
            .iter()
            .filter(|member| !member.me && member.pubkey != self.me)
            .cloned()
            .collect();
        if members.is_empty() {
            self.set_status("当前没有其他在线成员", true);
            return;
        }
        let selected = vec![false; members.len()];
        self.file_select = Some(FileSelect {
            members,
            selected,
            index: 0,
        });
    }

    fn on_file_select_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let Some(select) = self.file_select.as_mut() else {
            return;
        };
        match key.code {
            KeyCode::Up => {
                if select.index > 0 {
                    select.index -= 1;
                }
            }
            KeyCode::Down => {
                if select.index + 1 < select.members.len() {
                    select.index += 1;
                }
            }
            KeyCode::Char(' ') => {
                if let Some(flag) = select.selected.get_mut(select.index) {
                    *flag = !*flag;
                }
            }
            KeyCode::Enter => {
                let targets: Vec<String> = select
                    .members
                    .iter()
                    .zip(select.selected.iter())
                    .filter(|(_, chosen)| **chosen)
                    .map(|(member, _)| member.peer_id.clone())
                    .collect();
                if targets.is_empty() {
                    self.set_status("至少选择一名成员", true);
                    return;
                }
                let count = targets.len();
                self.input = format!("/send {} ", targets.join(","));
                self.cursor = self.input.chars().count();
                self.file_select = None;
                self.set_status(
                    format!("已选择 {count} 人，输入文件路径（Tab 补全）后回车发送"),
                    false,
                );
            }
            KeyCode::Esc => self.file_select = None,
            KeyCode::Char('c') if ctrl => self.should_quit = true,
            _ => {}
        }
    }

    fn handle_call(&mut self, rest: &str) {
        let arg = rest.trim();
        if arg.is_empty() {
            self.push_system(
                "通话：/call <昵称> 发起；来电时 /call accept 或 /call reject；\
                 通话中 /call mute、/call hangup，按住说话模式用 /call ptt 切换。",
            );
            return;
        }
        match arg {
            "accept" => {
                if let Some(incoming) = self.incoming_call.take() {
                    let _ = self.cmd.send(Cmd::AcceptCall {
                        call_id: incoming.call_id,
                    });
                    self.set_status("接听中…", false);
                } else {
                    self.set_status("没有待接听的来电", true);
                }
            }
            "reject" => {
                if let Some(incoming) = self.incoming_call.take() {
                    let _ = self.cmd.send(Cmd::RejectCall {
                        call_id: incoming.call_id,
                    });
                    self.set_status("已拒绝来电", false);
                } else {
                    self.set_status("没有待接听的来电", true);
                }
            }
            "hangup" => {
                let _ = self.cmd.send(Cmd::HangupCall);
                self.set_status("正在挂断…", false);
            }
            "mute" => {
                let muted = if let Some(call) = self.active_call.as_mut() {
                    call.muted = !call.muted;
                    Some(call.muted)
                } else {
                    None
                };
                match muted {
                    Some(muted) => self.set_status(
                        if muted {
                            "通话已静音"
                        } else {
                            "通话已取消静音"
                        },
                        false,
                    ),
                    None => self.set_status("当前没有通话", true),
                }
            }
            "ptt" => {
                if self.active_call.is_none() {
                    self.set_status("当前没有通话", true);
                } else if !self.voice.push_to_talk {
                    self.set_status("配置 voice.push_to_talk = true 后使用按住说话", true);
                } else {
                    self.ptt_active = !self.ptt_active;
                    self.set_status(
                        if self.ptt_active {
                            "按住说话：开"
                        } else {
                            "按住说话：关"
                        },
                        false,
                    );
                }
            }
            target => {
                if self.active_call.is_some() || self.incoming_call.is_some() {
                    self.set_status("已有通话或来电", true);
                    return;
                }
                let needle = target.trim_start_matches('@').to_lowercase();
                let found = self.members.iter().find(|member| {
                    !member.me
                        && (member.nick.to_lowercase() == needle
                            || member.pubkey.starts_with(&needle))
                });
                match found {
                    Some(member) => {
                        let _ = self.cmd.send(Cmd::Call {
                            target: member.pubkey.clone(),
                        });
                        self.set_status(format!("正在呼叫 {}…", member.nick), false);
                    }
                    None => self.set_status(format!("找不到在线成员: {target}"), true),
                }
            }
        }
    }

    fn handle_voice(&mut self, rest: &str) {
        let mut parts = rest.splitn(2, char::is_whitespace);
        let action = parts.next().unwrap_or("").trim();
        let arg = parts.next().unwrap_or("").trim();
        match action {
            "join" => {
                if self.voice_room_joined {
                    self.set_status("已在语音房中", false);
                } else {
                    self.toggle_voice_room();
                }
            }
            "leave" => {
                if self.voice_room_joined {
                    let _ = self.cmd.send(Cmd::VoiceRoomLeave);
                    self.voice_room_joined = false;
                    self.voice_room_audio = None;
                    self.set_status("正在退出语音房…", false);
                } else {
                    self.set_status("当前不在语音房", true);
                }
            }
            "mute" => {
                if self.voice_room_joined {
                    self.voice_room_muted = !self.voice_room_muted;
                    self.set_status(
                        if self.voice_room_muted {
                            "语音房已静音"
                        } else {
                            "语音房已取消静音"
                        },
                        false,
                    );
                } else {
                    self.set_status("当前不在语音房", true);
                }
            }
            "members" => {
                if !self.voice_room_joined {
                    self.set_status("当前不在语音房", true);
                } else {
                    let mut lines = vec![format!("语音房 {} 人：", self.voice_room_members.len())];
                    for member in &self.voice_room_members {
                        lines.push(format!(
                            "- {} ({})",
                            member.nick,
                            util::short_id(&member.pubkey)
                        ));
                    }
                    lines.push("/voice leave 退出，v 也可切换".to_string());
                    self.push_system(lines.join("\n"));
                }
            }
            "play" => {
                if arg.is_empty() {
                    self.set_status("用法: /voice play <WAV 路径>", true);
                    return;
                }
                let path = crate::util::expand_tilde(arg.trim_matches('"').trim_matches('\''));
                match crate::audio::play_file(&path) {
                    Ok(_) => self.set_status(format!("正在播放 {}", path.display()), false),
                    Err(err) => self.set_status(format!("播放失败: {err}"), true),
                }
            }
            "stop" => {
                let Some(mut message) = self.voice_message.take() else {
                    self.set_status("当前没有录音", true);
                    return;
                };
                crate::audio::stop_recording(&mut message.child);
                let ok = message
                    .path
                    .metadata()
                    .map(|meta| meta.len() > 44)
                    .unwrap_or(false);
                if !ok {
                    self.set_status("录音为空或失败", true);
                    return;
                }
                let _ = self.cmd.send(Cmd::SendFile {
                    target: message.target,
                    path: message.path.clone(),
                });
                self.set_status(format!("语音消息已发送: {}", message.path.display()), false);
            }
            "" => {
                self.push_system(
                    "语音消息：/voice <昵称> 开始录音，/voice stop 停止并发送；\
                     /voice play <路径> 播放。\n\
                     频道语音房：/voice join 加入，/voice leave 退出，/voice mute 静音，\
                     /voice members 看成员；浏览模式按 v 也可切换。",
                );
            }
            target => {
                if self.active_call.is_some() {
                    self.set_status("通话中不能录制语音消息，请先挂断", true);
                    return;
                }
                if self.voice_room_joined {
                    self.set_status("语音房中不能录制语音消息，请先 /voice leave", true);
                    return;
                }
                if self.voice_message.is_some() {
                    self.set_status("已有录音进行中，先 /voice stop", true);
                    return;
                }
                let needle = target.trim_start_matches('@').to_lowercase();
                let Some(member) = self.members.iter().find(|member| {
                    !member.me
                        && (member.nick.to_lowercase() == needle
                            || member.pubkey.starts_with(&needle))
                }) else {
                    self.set_status(format!("找不到在线成员: {target}"), true);
                    return;
                };
                if let Err(err) = crate::audio::check_tools() {
                    self.set_status(format!("语音工具不可用: {err}"), true);
                    return;
                }
                let dir = crate::audio::voice_dir(&self.dirs, &self.voice);
                let path = dir.join(format!("voice-{}.wav", crate::util::now_ms()));
                match crate::audio::start_recording(&self.voice, &path) {
                    Ok(child) => {
                        self.voice_message = Some(VoiceMessage {
                            child,
                            path: path.clone(),
                            target: member.pubkey.clone(),
                        });
                        self.set_status(
                            format!("录音中 → {}；输入 /voice stop 结束", member.nick),
                            false,
                        );
                    }
                    Err(err) => self.set_status(format!("开始录音失败: {err}"), true),
                }
            }
        }
    }

    fn et_profile(&self) -> crate::config::GroupProfile {
        crate::config::GroupProfile::load_at(&self.group_path).unwrap_or_default()
    }

    fn et_selected(&self) -> Option<String> {
        let profile = self.et_profile();
        crate::easytier::selected_node(&self.config_nodes, profile.et_node.as_deref())
    }

    fn reload_et_nodes(&mut self) {
        if let Ok(config) = crate::config::Config::ensure(&self.dirs) {
            self.config_nodes = crate::easytier::normalize_nodes(config.easytier.nodes.iter());
        }
    }

    fn restart_for_et(&mut self) {
        self.action = Some(TuiAction::Switch(self.room_id.clone()));
    }

    fn handle_easytier(&mut self, rest: &str) {
        let mut parts = rest.split_whitespace();
        let action = parts.next().unwrap_or("status");
        let args: Vec<&str> = parts.collect();
        match action {
            "" | "status" => {
                let selected = self.et_selected();
                let mut lines = Vec::new();
                if self.easytier.running {
                    let ip = self
                        .easytier
                        .ip
                        .map(|ip| ip.to_string())
                        .unwrap_or_else(|| "-".to_string());
                    lines.push(format!("EasyTier: 运行中 · 虚拟 IP {ip}"));
                    if let Some(node) = &selected {
                        lines.push(format!("当前节点: {node}"));
                    }
                } else if let Some(err) = &self.easytier.error {
                    lines.push(format!("EasyTier: 未运行（{err}）"));
                } else {
                    lines.push("EasyTier: 未开启（/et on 为当前频道开启）".to_string());
                }
                if self.config_nodes.is_empty() {
                    lines.push("节点池: 空（/et add <地址> 添加）".to_string());
                } else {
                    lines.push(format!(
                        "节点池: {} 个{}",
                        self.config_nodes.len(),
                        selected
                            .as_ref()
                            .map(|node| format!(" · 本频道选中 {node}"))
                            .unwrap_or_default()
                    ));
                }
                lines.push(
                    "命令: /et list · /et use <编号> · /et test [编号] · /et remove <编号>"
                        .to_string(),
                );
                lines.push("提示：修改节点或开关后会自动重启当前频道。".to_string());
                self.push_system(lines.join("\n"));
            }
            "on" | "off" => {
                let enabled = action == "on";
                if enabled && self.config_nodes.is_empty() {
                    self.set_status("没有 EasyTier 节点，先 /et add <地址>", true);
                    return;
                }
                match self.set_easytier_enabled(enabled) {
                    Ok(()) => {
                        self.push_system(format!(
                            "EasyTier 已{}，正在重启频道…",
                            if enabled { "开启" } else { "关闭" }
                        ));
                        self.restart_for_et();
                    }
                    Err(err) => self.set_status(format!("保存 EasyTier 开关失败: {err}"), true),
                }
            }
            "add" => {
                if args.is_empty() {
                    self.set_status("用法: /et add <tcp://host:port> [更多地址…]", true);
                    return;
                }
                let mut config = match crate::config::Config::ensure(&self.dirs) {
                    Ok(config) => config,
                    Err(err) => {
                        self.set_status(format!("读取配置失败: {err}"), true);
                        return;
                    }
                };
                let mut added = Vec::new();
                for raw in &args {
                    let node = match crate::easytier::normalize_node(raw) {
                        Ok(node) => node,
                        Err(err) => {
                            self.set_status(format!("节点地址无效（{raw}）: {err}"), true);
                            return;
                        }
                    };
                    if !config.easytier.nodes.contains(&node) {
                        config.easytier.nodes.push(node.clone());
                        added.push(node);
                    }
                }
                if added.is_empty() {
                    self.set_status("这些节点已经在列表中", false);
                    return;
                }
                if let Err(err) = config.save(&crate::config::config_path(&self.dirs)) {
                    self.set_status(format!("保存节点失败: {err}"), true);
                    return;
                }
                self.config_nodes = crate::easytier::normalize_nodes(config.easytier.nodes.iter());
                let mut profile = self.et_profile();
                if profile.et_node.is_none() {
                    profile.et_node = crate::easytier::selected_node(&self.config_nodes, None);
                    let _ = profile.save_at(&self.group_path);
                }
                self.push_system(format!(
                    "已添加 {} 个 EasyTier 节点：{}\n用 /et list 查看，/et use <编号> 选择本频道使用的节点。",
                    added.len(),
                    added.join(", ")
                ));
                if self.easytier.running {
                    self.push_system("EasyTier 正在运行，自动重启频道以应用新节点…".to_string());
                    self.restart_for_et();
                }
            }
            "remove" => {
                let Some(index) = args.first().and_then(|text| text.parse::<usize>().ok()) else {
                    self.set_status("用法: /et remove <编号>（/et list 查看编号）", true);
                    return;
                };
                if index == 0 || index > self.config_nodes.len() {
                    self.set_status("节点编号超出范围", true);
                    return;
                }
                let removed = self.config_nodes[index - 1].clone();
                let mut config = match crate::config::Config::ensure(&self.dirs) {
                    Ok(config) => config,
                    Err(err) => {
                        self.set_status(format!("读取配置失败: {err}"), true);
                        return;
                    }
                };
                config.easytier.nodes.retain(|node| node != &removed);
                if let Err(err) = config.save(&crate::config::config_path(&self.dirs)) {
                    self.set_status(format!("保存节点失败: {err}"), true);
                    return;
                }
                // 同时清理所有频道里残留的旧节点和选中项，避免重启后“复活”。
                let purged = crate::rooms::purge_easytier_node(&self.dirs, &removed).unwrap_or(0);
                self.reload_et_nodes();
                let mut profile = self.et_profile();
                profile.et_node =
                    crate::easytier::selected_node(&self.config_nodes, profile.et_node.as_deref());
                let _ = profile.save_at(&self.group_path);
                self.push_system(format!(
                    "已移除 EasyTier 节点: {removed}（清理 {purged} 处频道配置）{}",
                    if self.config_nodes.is_empty() {
                        "\n节点池已空；/et on 将无法启动 EasyTier。"
                    } else {
                        ""
                    }
                ));
                if self.easytier.running {
                    self.restart_for_et();
                }
            }
            "use" | "select" => {
                let Some(index) = args.first().and_then(|text| text.parse::<usize>().ok()) else {
                    self.set_status("用法: /et use <编号>（/et list 查看编号）", true);
                    return;
                };
                if index == 0 || index > self.config_nodes.len() {
                    self.set_status("节点编号超出范围", true);
                    return;
                }
                let node = self.config_nodes[index - 1].clone();
                let mut profile = self.et_profile();
                profile.et_node = Some(node.clone());
                if let Err(err) = profile.save_at(&self.group_path) {
                    self.set_status(format!("保存节点选择失败: {err}"), true);
                    return;
                }
                self.push_system(format!("本频道将使用 EasyTier 节点: {node}\n正在重启频道…"));
                self.restart_for_et();
            }
            "test" => {
                let targets: Vec<(usize, String)> = match args.first() {
                    Some(text) => match text.parse::<usize>() {
                        Ok(index) if index > 0 && index <= self.config_nodes.len() => {
                            vec![(index, self.config_nodes[index - 1].clone())]
                        }
                        _ => {
                            self.set_status("用法: /et test [编号]（省略编号测试全部）", true);
                            return;
                        }
                    },
                    None => self
                        .config_nodes
                        .iter()
                        .enumerate()
                        .map(|(index, node)| (index + 1, node.clone()))
                        .collect(),
                };
                if targets.is_empty() {
                    self.set_status("还没有 EasyTier 节点，先 /et add <地址>", true);
                    return;
                }
                let mut lines = vec!["EasyTier 节点测试（TCP 连接，超时 2 秒）：".to_string()];
                for (index, node) in targets {
                    let probe = crate::easytier::probe_node(&node, Duration::from_secs(2));
                    let result = if probe.ok {
                        match probe.latency_ms {
                            Some(ms) => format!("通 · {ms} ms"),
                            None => "通".to_string(),
                        }
                    } else {
                        format!("不通 · {}", probe.detail)
                    };
                    lines.push(format!("{index}. {node} · {result}"));
                }
                self.push_system(lines.join("\n"));
            }
            "list" => {
                if self.config_nodes.is_empty() {
                    self.push_system("尚未添加 EasyTier 节点；用 /et add <tcp://host:port> 添加。");
                } else {
                    let selected = self.et_selected();
                    let mut lines = vec![format!(
                        "EasyTier 节点池（{} 个）：",
                        self.config_nodes.len()
                    )];
                    for (index, node) in self.config_nodes.iter().enumerate() {
                        let mark = if selected.as_deref() == Some(node.as_str()) {
                            "*"
                        } else {
                            " "
                        };
                        let running = if self.easytier.nodes.contains(node) {
                            " 运行中"
                        } else {
                            ""
                        };
                        lines.push(format!("{mark} {}. {node}{running}", index + 1));
                    }
                    lines.push(
                        "使用 /et use <编号> 选择本频道节点；/et test 测试连通性。".to_string(),
                    );
                    self.push_system(lines.join("\n"));
                }
            }
            _ => self.set_status(
                "用法: /et [status|on|off|add <地址>|remove <编号>|use <编号>|test [编号]|list]",
                true,
            ),
        }
    }

    fn set_easytier_enabled(&mut self, enabled: bool) -> Result<()> {
        let mut profile =
            crate::config::GroupProfile::load_at(&self.group_path).unwrap_or_default();
        profile.easytier_enabled = Some(enabled);
        profile.save_at(&self.group_path)?;
        Ok(())
    }

    fn copy_to_input(&mut self) {
        if let Some(text) = self.last_copy.clone() {
            self.input = text;
            self.cursor = self.input.chars().count();
            self.completion = None;
            self.set_status("已放入输入行，可直接复制", false);
        } else {
            self.set_status("暂无可复制内容", true);
        }
    }

    fn copy_to_clipboard(&mut self) {
        let Some(text) = self.last_copy.clone() else {
            self.set_status("暂无可复制内容", true);
            return;
        };
        // OSC 52：让终端把文本写入系统剪贴板；不支持的终端可以继续用 Ctrl+Y 放入输入行。
        let encoded = crate::crypto::b64e(text.as_bytes());
        let sequence = format!("\x1b]52;c;{encoded}\x07");
        let mut stdout = std::io::stdout();
        match stdout
            .write_all(sequence.as_bytes())
            .and_then(|_| stdout.flush())
        {
            Ok(()) => self.set_status(
                "已尝试复制到系统剪贴板；若终端不支持 OSC 52，按 Ctrl+Y 放入输入行",
                false,
            ),
            Err(err) => self.set_status(
                format!("复制到剪贴板失败: {err}；按 Ctrl+Y 放入输入行"),
                true,
            ),
        }
    }

    // ------------------------------------------------------------ 回复

    fn start_reply_select(&mut self) {
        let Some(idx) = self.messages.iter().rposition(|m| m.kind == Kind::Chat) else {
            self.set_status("暂无可回复的消息", true);
            return;
        };
        self.reply_select = Some(idx);
        self.follow = false;
    }

    fn move_reply_select(&mut self, delta: isize) {
        let Some(mut idx) = self.reply_select else {
            return;
        };
        loop {
            if delta < 0 {
                if idx == 0 {
                    return;
                }
                idx -= 1;
            } else {
                if idx + 1 >= self.messages.len() {
                    return;
                }
                idx += 1;
            }
            if self.messages[idx].kind == Kind::Chat {
                self.reply_select = Some(idx);
                return;
            }
        }
    }

    fn confirm_reply_select(&mut self) {
        let Some(idx) = self.reply_select.take() else {
            return;
        };
        self.set_reply_target(idx);
    }

    fn set_reply_target(&mut self, idx: usize) {
        let Some(message) = self.messages.get(idx) else {
            return;
        };
        if message.kind != Kind::Chat {
            return;
        }
        let reply = ReplyRef {
            id: message.id.clone(),
            nick: message.nick.clone(),
            preview: truncate_width(&message.text, 40),
        };
        let seq = message.seq;
        let nick = message.nick.clone();
        self.pending_reply = Some((seq, reply));
        self.set_status(format!("回复 #{seq} {nick}"), false);
        self.follow = true;
        self.scroll = 0;
    }

    fn command_reply(&mut self, arg: &str) {
        let arg = arg.trim().trim_start_matches('#');
        if arg.is_empty() {
            let lines: Vec<String> = self
                .messages
                .iter()
                .rev()
                .filter(|m| m.kind == Kind::Chat)
                .take(20)
                .map(|m| format!("#{} {}: {}", m.seq, m.nick, truncate_width(&m.text, 50)))
                .collect();
            if lines.is_empty() {
                self.set_status("暂无可回复的消息", true);
            } else {
                self.push_system(format!(
                    "输入 /reply <编号> 回复指定消息：\n{}",
                    lines.join("\n")
                ));
            }
            return;
        }
        let Ok(seq) = arg.parse::<u64>() else {
            self.set_status("用法: /reply <编号>（输入 /reply 查看编号）", true);
            return;
        };
        match self
            .messages
            .iter()
            .position(|m| m.seq == seq && m.kind == Kind::Chat)
        {
            Some(idx) => self.set_reply_target(idx),
            None => self.set_status(format!("找不到编号 #{seq} 的消息"), true),
        }
    }

    // ------------------------------------------------------------ 补全

    fn complete(&mut self, reverse: bool) {
        if let Some(mut state) = self.completion.take() {
            let len = state.candidates.len();
            let unchanged = state.end == self.cursor
                && state.start <= state.end
                && state.end <= self.input.chars().count();
            if len > 0 && unchanged {
                state.index = if reverse {
                    (state.index + len - 1) % len
                } else {
                    (state.index + 1) % len
                };
                self.apply_completion(state);
                return;
            }
        }
        let Some((start, candidates)) = self.completion_context() else {
            return;
        };
        if candidates.is_empty() {
            return;
        }
        let state = Completion {
            start,
            end: self.cursor,
            candidates,
            index: 0,
        };
        self.apply_completion(state);
    }

    fn apply_completion(&mut self, mut state: Completion) {
        let candidate = state.candidates[state.index].clone();
        let dir = candidate.ends_with('/') || candidate.ends_with('\\');
        let value = if dir {
            candidate
        } else {
            format!("{candidate} ")
        };
        let start = byte_index(&self.input, state.start);
        let end = byte_index(&self.input, state.end);
        self.input.replace_range(start..end, &value);
        let end = state.start + value.chars().count();
        self.cursor = end;
        state.end = end;
        self.completion = Some(state);
    }

    /// 计算当前光标处的补全范围与候选。
    fn completion_context(&self) -> Option<(usize, Vec<String>)> {
        let before: Vec<char> = self.input.chars().take(self.cursor).collect();
        let pad = before.iter().take_while(|ch| ch.is_whitespace()).count();
        let lead: String = before[pad..].iter().collect();
        let token_start = before
            .iter()
            .rposition(|ch| ch.is_whitespace())
            .map(|idx| idx + 1)
            .unwrap_or(0);
        let token: String = before[token_start..].iter().collect();
        let token_lower = token.to_lowercase();

        // 指令名补全：整个输入还只有一个以 / 开头的词。
        if lead.starts_with('/') && !lead.chars().any(char::is_whitespace) {
            let candidates = COMMANDS
                .iter()
                .filter(|cmd| cmd.to_lowercase().starts_with(&token_lower))
                .map(|cmd| cmd.to_string())
                .collect();
            return Some((token_start, candidates));
        }

        // @ 提及补全。
        if let Some(query) = token.strip_prefix('@') {
            let query = query.to_lowercase();
            let mut candidates = Vec::new();
            let mut seen = std::collections::HashSet::new();
            for member in &self.members {
                if member.me || member.nick.is_empty() {
                    continue;
                }
                if member.nick.to_lowercase().starts_with(&query)
                    && seen.insert(member.nick.clone())
                {
                    candidates.push(format!("@{}", member.nick));
                }
            }
            if "所有人".starts_with(&query) && seen.insert("所有人".to_string()) {
                candidates.push("@所有人".to_string());
            }
            return Some((token_start, candidates));
        }

        // 指令参数补全。
        let before_token: String = before[..token_start].iter().collect();
        let parts: Vec<&str> = before_token.split_whitespace().collect();
        match parts.as_slice() {
            [cmd] if cmd.eq_ignore_ascii_case("/send") => {
                let candidates = self
                    .members
                    .iter()
                    .filter(|m| !m.me && !m.nick.is_empty())
                    .filter(|m| m.nick.to_lowercase().starts_with(&token_lower))
                    .map(|m| m.nick.clone())
                    .collect();
                Some((token_start, candidates))
            }
            [cmd] if cmd.eq_ignore_ascii_case("/accept") || cmd.eq_ignore_ascii_case("/deny") => {
                let candidates = self
                    .offers
                    .iter()
                    .map(|o| util::short_id(&o.xfer))
                    .filter(|id| id.starts_with(&token))
                    .collect();
                Some((token_start, candidates))
            }
            [cmd, _targets] if cmd.eq_ignore_ascii_case("/send") => {
                Some((token_start, path_candidates(&token)))
            }
            [cmd] if cmd.eq_ignore_ascii_case("/et") => {
                let candidates = [
                    "status", "on", "off", "add", "list", "remove", "use", "test",
                ]
                .iter()
                .filter(|sub| sub.starts_with(&token_lower))
                .map(|sub| sub.to_string())
                .collect();
                Some((token_start, candidates))
            }
            [cmd, sub]
                if cmd.eq_ignore_ascii_case("/et")
                    && (sub.eq_ignore_ascii_case("remove")
                        || sub.eq_ignore_ascii_case("use")
                        || sub.eq_ignore_ascii_case("select")
                        || sub.eq_ignore_ascii_case("test")) =>
            {
                let candidates = self
                    .config_nodes
                    .iter()
                    .enumerate()
                    .map(|(idx, _node)| (idx + 1).to_string())
                    .filter(|index| index.starts_with(&token))
                    .collect();
                Some((token_start, candidates))
            }
            [cmd] if cmd.eq_ignore_ascii_case("/call") => {
                let mut candidates: Vec<String> = ["accept", "reject", "hangup", "mute", "ptt"]
                    .iter()
                    .filter(|sub| sub.starts_with(&token_lower))
                    .map(|sub| sub.to_string())
                    .collect();
                candidates.extend(
                    self.members
                        .iter()
                        .filter(|m| !m.me && !m.nick.is_empty())
                        .filter(|m| m.nick.to_lowercase().starts_with(&token_lower))
                        .map(|m| m.nick.clone()),
                );
                Some((token_start, candidates))
            }
            [cmd] if cmd.eq_ignore_ascii_case("/voice") => {
                let mut candidates: Vec<String> = ["join", "leave", "play", "stop"]
                    .iter()
                    .filter(|sub| sub.starts_with(&token_lower))
                    .map(|sub| sub.to_string())
                    .collect();
                candidates.extend(
                    self.members
                        .iter()
                        .filter(|m| !m.me && !m.nick.is_empty())
                        .filter(|m| m.nick.to_lowercase().starts_with(&token_lower))
                        .map(|m| m.nick.clone()),
                );
                Some((token_start, candidates))
            }
            [cmd, sub]
                if cmd.eq_ignore_ascii_case("/voice") && sub.eq_ignore_ascii_case("play") =>
            {
                Some((token_start, path_candidates(&token)))
            }
            [cmd] if cmd.eq_ignore_ascii_case("/switch") => {
                let candidates = self
                    .groups
                    .iter()
                    .map(|group| {
                        if group.name.is_empty() {
                            group.id.clone()
                        } else {
                            group.name.clone()
                        }
                    })
                    .filter(|name| name.to_lowercase().starts_with(&token_lower))
                    .collect();
                Some((token_start, candidates))
            }
            [cmd] if cmd.eq_ignore_ascii_case("/reply") => {
                let candidates = self
                    .messages
                    .iter()
                    .rev()
                    .filter(|m| m.kind == Kind::Chat)
                    .take(50)
                    .map(|m| m.seq.to_string())
                    .filter(|seq| seq.starts_with(&token))
                    .collect();
                Some((token_start, candidates))
            }
            _ => None,
        }
    }

    fn insert_text(&mut self, text: &str) {
        // 只去掉控制字符，保留空格（sanitize_text 会 trim，不适合逐字输入）。
        let cleaned: String = text
            .chars()
            .filter_map(|ch| {
                if ch == '\n' || ch == '\t' {
                    Some(' ')
                } else if ch.is_control() {
                    None
                } else {
                    Some(ch)
                }
            })
            .collect();
        if cleaned.is_empty() {
            return;
        }
        let byte = byte_index(&self.input, self.cursor);
        self.input.insert_str(byte, &cleaned);
        self.cursor += cleaned.chars().count();
    }

    fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let end = byte_index(&self.input, self.cursor);
        let start = byte_index(&self.input, self.cursor - 1);
        self.input.replace_range(start..end, "");
        self.cursor -= 1;
    }

    fn delete_forward(&mut self) {
        if self.cursor >= self.input.chars().count() {
            return;
        }
        let start = byte_index(&self.input, self.cursor);
        let end = byte_index(&self.input, self.cursor + 1);
        self.input.replace_range(start..end, "");
    }

    fn history_prev(&mut self) {
        if self.history.is_empty() {
            return;
        }
        let next = match self.hist_pos {
            None => self.history.len() - 1,
            Some(0) => 0,
            Some(pos) => pos - 1,
        };
        self.hist_pos = Some(next);
        self.input = self.history[next].clone();
        self.cursor = self.input.chars().count();
    }

    fn history_next(&mut self) {
        let Some(pos) = self.hist_pos else {
            return;
        };
        if pos + 1 >= self.history.len() {
            self.hist_pos = None;
            self.input.clear();
            self.cursor = 0;
        } else {
            self.hist_pos = Some(pos + 1);
            self.input = self.history[pos + 1].clone();
            self.cursor = self.input.chars().count();
        }
    }

    fn submit(&mut self) {
        let text = self.input.trim().to_string();
        self.input.clear();
        self.cursor = 0;
        self.hist_pos = None;
        self.scroll = 0;
        self.follow = true;
        self.completion = None;
        if text.is_empty() {
            return;
        }
        self.history.push(text.clone());
        if self.history.len() > 300 {
            self.history.remove(0);
        }
        if text.starts_with('/') {
            self.command(&text);
        } else {
            let reply = self.pending_reply.take().map(|(_, reply)| reply);
            let _ = self.cmd.send(Cmd::Chat { text, reply });
        }
    }

    fn command(&mut self, text: &str) {
        let body = text.trim_start_matches('/');
        let mut split = body.splitn(2, char::is_whitespace);
        let name = split.next().unwrap_or("").to_ascii_lowercase();
        let rest = split.next().unwrap_or("").trim().to_string();
        match name.as_str() {
            "help" | "?" => self.push_system(HELP),
            "name" => {
                if rest.is_empty() {
                    self.set_status("用法: /name <昵称>", true);
                } else {
                    let _ = self.cmd.send(Cmd::SetNick(rest));
                }
            }
            "rename" => {
                if rest.is_empty() {
                    let current = self
                        .room_name
                        .clone()
                        .unwrap_or_else(|| format!("频道 {}", self.room_id));
                    self.set_status(
                        format!("当前频道名称: {current}（用法 /rename <名称>）"),
                        false,
                    );
                } else {
                    let name: String = util::sanitize_text(&rest).chars().take(40).collect();
                    if name.is_empty() {
                        self.set_status("频道名称不能为空", true);
                        return;
                    }
                    let mut profile =
                        crate::config::GroupProfile::load_at(&self.group_path).unwrap_or_default();
                    profile.name = Some(name.clone());
                    match profile.save_at(&self.group_path) {
                        Ok(()) => {
                            self.room_name = Some(name.clone());
                            for group in &mut self.groups {
                                if group.id == self.room_id {
                                    group.name = name.clone();
                                }
                            }
                            self.set_status(format!("频道名称已改为 {name}（仅本机显示）"), false);
                        }
                        Err(err) => self.set_status(format!("保存频道名称失败: {err}"), true),
                    }
                }
            }
            "invite" => {
                if self.invite.is_empty() {
                    self.set_status("暂无邀请码", true);
                    return;
                }
                let text = format!(
                    "邀请码（已放入输入行，Ctrl+Shift+C 可复制）：\n{}\n对方运行: asa join '<邀请码>' --name 昵称",
                    self.invite
                );
                self.last_copy = Some(self.invite.clone());
                self.input = self.invite.clone();
                self.cursor = self.input.chars().count();
                self.completion = None;
                self.push_system(text);
                self.set_status("邀请码已放入输入行，可直接复制", false);
            }
            "peers" => {
                if self.members.is_empty() {
                    self.set_status("暂无成员信息", false);
                } else {
                    let mut lines = vec![format!("在线成员 {} 人：", self.members.len())];
                    for member in &self.members {
                        lines.push(format!(
                            "{} {}  {}  {}",
                            if member.host { "★" } else { " " },
                            member.nick,
                            member.addr.clone().unwrap_or_else(|| "-".to_string()),
                            if member.me { "(你)" } else { "" }
                        ));
                    }
                    self.push_system(lines.join("\n"));
                }
            }
            "connect" => {
                if rest.is_empty() {
                    self.set_status("用法: /connect <ip:port>", true);
                } else {
                    let _ = self.cmd.send(Cmd::Connect(rest));
                }
            }
            "send" => {
                if rest.trim().is_empty() {
                    self.start_file_select();
                    return;
                }
                let limit = if self.max_file == u64::MAX {
                    "不限".to_string()
                } else {
                    util::human_size(self.max_file)
                };
                let Some((targets_text, path_text)) = rest.split_once(char::is_whitespace) else {
                    self.input = format!("/send {} ", rest.trim());
                    self.cursor = self.input.chars().count();
                    self.completion = None;
                    self.set_status(
                        format!("输入文件路径（Tab 补全，上限 {limit}）后回车发送"),
                        false,
                    );
                    return;
                };
                let path = path_text.trim().trim_matches('"').trim_matches('\'');
                if targets_text.trim().is_empty() || path.is_empty() {
                    self.set_status(
                        format!("用法: /send <成员1,成员2|@all> <文件路径>（上限 {limit}）"),
                        true,
                    );
                    return;
                }
                let targets = self.resolve_send_targets(targets_text);
                if targets.is_empty() {
                    self.set_status("没有匹配的在线成员", true);
                    return;
                }
                let files = expand_send_files(path, 200);
                if files.is_empty() {
                    self.set_status(format!("找不到可发送的文件: {path}"), true);
                    return;
                }
                for file in &files {
                    let _ = self.cmd.send(Cmd::SendFileMany {
                        targets: targets.clone(),
                        path: file.clone(),
                    });
                }
                self.push_system(format!(
                    "已发起 {} 个文件传输给 {} 人{}",
                    files.len(),
                    targets.len(),
                    if files.len() >= 200 {
                        "（达到 200 个上限，已截断）"
                    } else {
                        ""
                    }
                ));
            }
            "copy" => self.copy_to_clipboard(),
            "mouse" => {
                self.mouse_capture = match rest.trim() {
                    "on" => true,
                    "off" => false,
                    "" => !self.mouse_capture,
                    _ => {
                        self.set_status("用法: /mouse [on|off]", true);
                        return;
                    }
                };
                self.set_status(
                    if self.mouse_capture {
                        "鼠标捕获已开启：滚轮可滚动；Shift+拖选仍可选择文本"
                    } else {
                        "鼠标捕获已关闭：可直接拖选文本；/mouse on 恢复滚轮"
                    },
                    false,
                );
            }
            "sync" => {
                let _ = self.cmd.send(Cmd::Sync);
            }
            "call" => self.handle_call(rest.trim()),
            "voice" => self.handle_voice(rest.trim()),
            "channels" => {
                self.action = Some(TuiAction::Lobby);
            }
            "switch" => {
                if rest.is_empty() {
                    self.action = Some(TuiAction::Lobby);
                } else {
                    let needle = rest.to_lowercase();
                    let found = self.groups.iter().find(|group| {
                        group.id.starts_with(&needle)
                            || group.name.to_lowercase() == needle
                            || group.name.to_lowercase().contains(&needle)
                    });
                    match found {
                        Some(group) => self.action = Some(TuiAction::Switch(group.id.clone())),
                        None => self.set_status(format!("找不到频道: {rest}"), true),
                    }
                }
            }
            "et" => self.handle_easytier(&rest),
            "accept" => self.resolve_offer(&rest, true),
            "deny" | "reject" => self.resolve_offer(&rest, false),
            "files" => {
                if self.transfers.is_empty() {
                    self.set_status("当前没有进行中的文件传输", false);
                } else {
                    let mut lines = Vec::new();
                    for item in &self.transfers {
                        let percent = if item.total == 0 {
                            0
                        } else {
                            (item.done.saturating_mul(100) / item.total).min(100)
                        };
                        lines.push(format!(
                            "{} {}  {}%  ({}/{})",
                            if item.outgoing { "↑" } else { "↓" },
                            item.name,
                            percent,
                            util::human_size(item.done),
                            util::human_size(item.total)
                        ));
                    }
                    self.push_system(lines.join("\n"));
                }
            }
            "clear" => {
                self.messages.clear();
                self.pending_reply = None;
                self.reply_select = None;
            }
            "reply" => self.command_reply(&rest),
            "quit" | "exit" => self.should_quit = true,
            "" => {}
            other => self.set_status(format!("未知命令 /{other}，输入 /help 查看帮助"), true),
        }
    }

    fn resolve_offer(&mut self, needle: &str, accept: bool) {
        let needle = needle.trim();
        let target = if needle.is_empty() {
            if self.offers.len() == 1 {
                Some(self.offers[0].xfer.clone())
            } else {
                None
            }
        } else {
            self.offers
                .iter()
                .find(|o| {
                    o.xfer == needle
                        || o.xfer.starts_with(needle)
                        || util::short_id(&o.xfer) == needle
                })
                .map(|o| o.xfer.clone())
        };
        let Some(xfer) = target else {
            let pending: Vec<String> = self
                .offers
                .iter()
                .map(|o| {
                    format!(
                        "{} · {} ({}) 来自 {}",
                        util::short_id(&o.xfer),
                        o.name,
                        util::human_size(o.size),
                        o.from_nick
                    )
                })
                .collect();
            if pending.is_empty() {
                self.set_status("当前没有待处理的文件请求", true);
            } else {
                self.push_system(format!("待处理请求：\n{}", pending.join("\n")));
            }
            return;
        };
        if accept {
            let _ = self.cmd.send(Cmd::Accept(xfer.clone()));
        } else {
            let _ = self.cmd.send(Cmd::Reject(xfer.clone()));
        }
        self.offers.retain(|o| o.xfer != xfer);
    }

    // ------------------------------------------------------------ 渲染

    fn draw(&mut self, frame: &mut Frame) {
        let area = frame.area();
        let reply_height = u16::from(self.pending_reply.is_some());
        if area.height < 6 + reply_height || area.width < 20 {
            frame.render_widget(
                Paragraph::new("终端窗口太小，请调大后使用（至少 20x7）"),
                area,
            );
            return;
        }
        self.wide_layout = area.width >= 76;
        let mut constraints = vec![
            Constraint::Length(1),
            Constraint::Min(3),
            Constraint::Length(1),
        ];
        if reply_height > 0 {
            constraints.push(Constraint::Length(1));
        }
        constraints.push(Constraint::Length(1));
        constraints.push(Constraint::Length(1));
        let chunks = Layout::new(Direction::Vertical, constraints).split(area);
        self.draw_header(frame, chunks[0]);
        self.draw_body(frame, chunks[1]);
        self.draw_file_select(frame, chunks[1]);
        self.draw_status(frame, chunks[2]);
        let mut idx = 3;
        if reply_height > 0 {
            self.draw_reply_bar(frame, chunks[idx]);
            idx += 1;
        }
        self.draw_input(frame, chunks[idx]);
        self.draw_footer(frame, chunks[idx + 1]);
        self.draw_completion_popup(frame, chunks[idx]);
    }

    fn draw_header(&self, frame: &mut Frame, area: Rect) {
        let host_nick = self
            .host
            .as_deref()
            .map(|pubkey| {
                self.members
                    .iter()
                    .find(|m| m.pubkey == pubkey)
                    .map(|m| m.nick.clone())
                    .unwrap_or_else(|| util::short_id(pubkey))
            })
            .unwrap_or_else(|| "-".to_string());
        let you_are_host = self.host.as_deref() == Some(self.me.as_str());
        let mut spans = vec![
            Span::styled(
                " ASA ",
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" 频道 ", Style::default().fg(Color::Cyan)),
            Span::styled(
                format!(
                    "{} ",
                    self.room_name
                        .as_deref()
                        .map(|name| name.chars().take(16).collect::<String>())
                        .filter(|name| !name.is_empty())
                        .unwrap_or_else(|| self.room_id.chars().take(8).collect())
                ),
                Style::default().fg(Color::Gray),
            ),
            Span::raw("· "),
            Span::styled(
                format!("服主 {host_nick}"),
                Style::default().fg(if you_are_host {
                    Color::Yellow
                } else {
                    Color::Gray
                }),
            ),
            Span::raw(" · "),
            Span::styled(
                format!("在线 {} 人 ", self.members.len().max(1)),
                Style::default().fg(Color::Gray),
            ),
            Span::raw("· "),
            Span::styled(
                format!("{}@{} ", self.nick, self.listen),
                Style::default().fg(Color::DarkGray),
            ),
        ];
        if self.members.len() <= 1 {
            spans.push(Span::styled(
                "(等待成员加入 · /invite 邀请)",
                Style::default().fg(Color::DarkGray),
            ));
        }
        frame.render_widget(Paragraph::new(Line::from(spans)), area);
    }

    fn draw_body(&mut self, frame: &mut Frame, area: Rect) {
        let (chat, side) = body_layout(area, self.wide_layout);
        self.draw_chat(frame, chat);
        if let Some((members, transfers)) = side {
            self.draw_members(frame, members);
            self.draw_transfers(frame, transfers);
        }
    }

    fn draw_chat(&mut self, frame: &mut Frame, area: Rect) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::DarkGray))
            .title(Line::from(vec![
                Span::styled(" 聊天 ", Style::default().fg(Color::Cyan)),
                Span::styled(
                    format!("{} 条 ", self.messages.len()),
                    Style::default().fg(Color::DarkGray),
                ),
            ]));
        let inner = block.inner(area);
        let width = inner.width.max(8) as usize;
        let (lines, starts) = self.chat_lines(width);
        let total = lines.len();
        let height = inner.height as usize;
        let max_scroll = total.saturating_sub(height);
        if self.scroll > max_scroll {
            self.scroll = max_scroll;
        }
        if let Some(sel) = self.reply_select {
            if let Some(&sel_start) = starts.get(sel) {
                let sel_end = starts
                    .get(sel + 1)
                    .copied()
                    .unwrap_or(lines.len())
                    .saturating_sub(1)
                    .max(sel_start);
                let view_start = total.saturating_sub(height + self.scroll);
                if sel_start < view_start {
                    self.scroll = total.saturating_sub(height + sel_start);
                } else if sel_end >= view_start + height {
                    self.scroll = total.saturating_sub(sel_end + 1);
                }
                if self.scroll > max_scroll {
                    self.scroll = max_scroll;
                }
            }
        }
        let start = total.saturating_sub(height + self.scroll);
        let paragraph = Paragraph::new(lines).block(block).scroll((start as u16, 0));
        frame.render_widget(paragraph, area);
        if total > height && area.width > 2 && area.height > 2 {
            let sb_area = Rect {
                x: area.x + area.width.saturating_sub(1),
                y: area.y + 1,
                width: 1,
                height: area.height.saturating_sub(2),
            };
            let mut state = ScrollbarState::new(total)
                .viewport_content_length(height)
                .position(start);
            frame.render_stateful_widget(
                Scrollbar::new(ScrollbarOrientation::VerticalRight)
                    .begin_symbol(None)
                    .end_symbol(None)
                    .track_symbol(Some("│"))
                    .thumb_symbol("█"),
                sb_area,
                &mut state,
            );
        }
    }

    fn chat_lines(&self, width: usize) -> (Vec<Line<'static>>, Vec<usize>) {
        let mut lines = Vec::new();
        let mut starts = Vec::with_capacity(self.messages.len());
        for (idx, message) in self.messages.iter().enumerate() {
            starts.push(lines.len());
            let first_line = lines.len();
            if let Some(reply) = &message.reply {
                let head = format!("  │ {}: ", truncate_width(&reply.nick, 12));
                let avail = width
                    .saturating_sub(UnicodeWidthStr::width(head.as_str()))
                    .max(4);
                lines.push(Line::from(Span::styled(
                    format!("{head}{}", truncate_width(&reply.preview, avail)),
                    Style::default().fg(Color::DarkGray),
                )));
            }
            let mentioned = message.kind == Kind::Chat
                && message.mentions.iter().any(|pubkey| pubkey == &self.me);
            match message.kind {
                Kind::Chat => {
                    let mark = if mentioned { "@ " } else { "" };
                    let prefix = format!("{}{} {}: ", mark, util::hhmm(message.ts), message.nick);
                    let text_style = if mentioned {
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::White)
                    };
                    let mut prefix_style = Style::default().fg(if mentioned {
                        Color::Yellow
                    } else {
                        nick_color(&message.nick)
                    });
                    if message.mine || mentioned {
                        prefix_style = prefix_style.add_modifier(Modifier::BOLD);
                    }
                    wrap_with_prefix(
                        &prefix,
                        prefix_style,
                        &message.text,
                        text_style,
                        width,
                        &mut lines,
                    );
                }
                Kind::System => {
                    let prefix = format!("{} · ", util::hhmm(message.ts));
                    wrap_with_prefix(
                        &prefix,
                        Style::default().fg(Color::DarkGray),
                        &message.text,
                        Style::default().fg(Color::Gray),
                        width,
                        &mut lines,
                    );
                }
            }
            if self.reply_select == Some(idx) {
                for line in &mut lines[first_line..] {
                    for span in &mut line.spans {
                        span.style = span.style.add_modifier(Modifier::REVERSED);
                    }
                }
            }
        }
        (lines, starts)
    }

    fn draw_members(&self, frame: &mut Frame, area: Rect) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::DarkGray))
            .title(Line::from(vec![
                Span::styled(" 成员 ", Style::default().fg(Color::Cyan)),
                Span::styled(
                    format!("{} ", self.members.len()),
                    Style::default().fg(Color::DarkGray),
                ),
            ]));
        let items: Vec<ListItem> = self
            .members
            .iter()
            .map(|member| {
                let marker = if member.host { "★ " } else { "  " };
                let suffix = if member.me { " (你)" } else { "" };
                let color = if member.host {
                    Color::Yellow
                } else {
                    nick_color(&member.nick)
                };
                let line1 = Line::from(vec![
                    Span::styled(marker, Style::default().fg(Color::Yellow)),
                    Span::styled(
                        format!("{}{}", member.nick, suffix),
                        Style::default().fg(color).add_modifier(Modifier::BOLD),
                    ),
                ]);
                let addr = member.addr.clone().unwrap_or_else(|| "-".to_string());
                let line2 = Line::from(Span::styled(
                    format!(
                        "  {} · {}",
                        member.peer_id.chars().take(8).collect::<String>(),
                        addr
                    ),
                    Style::default().fg(Color::DarkGray),
                ));
                ListItem::new(vec![line1, line2])
            })
            .collect();
        frame.render_widget(List::new(items).block(block), area);
    }

    fn draw_transfers(&self, frame: &mut Frame, area: Rect) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::DarkGray))
            .title(Line::from(vec![
                Span::styled(" 传输 ", Style::default().fg(Color::Cyan)),
                Span::styled(
                    format!("{} ", self.transfers.len()),
                    Style::default().fg(Color::DarkGray),
                ),
            ]));
        if self.transfers.is_empty() {
            let hint = if self.offers.is_empty() {
                "空闲".to_string()
            } else {
                format!("{} 个文件请求待处理", self.offers.len())
            };
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    hint,
                    Style::default().fg(Color::DarkGray),
                )))
                .block(block),
                area,
            );
            return;
        }
        let items: Vec<ListItem> = self
            .transfers
            .iter()
            .take(area.height.saturating_sub(2) as usize)
            .map(|item| {
                let percent = if item.total == 0 {
                    0
                } else {
                    (item.done.saturating_mul(100) / item.total).min(100)
                };
                let bar_len = 10usize;
                let filled = (percent as usize * bar_len / 100).min(bar_len);
                let bar = format!("{}{}", "█".repeat(filled), "░".repeat(bar_len - filled));
                let line1 = Line::from(vec![
                    Span::styled(
                        format!("{} ", if item.outgoing { "↑" } else { "↓" }),
                        Style::default().fg(if item.outgoing {
                            Color::Magenta
                        } else {
                            Color::Cyan
                        }),
                    ),
                    Span::styled(format!("{percent:>3}% "), Style::default().fg(Color::Gray)),
                    Span::styled(bar, Style::default().fg(Color::Cyan)),
                ]);
                let line2 = Line::from(Span::styled(
                    format!(
                        "  {} · {}",
                        truncate(&item.name, 12),
                        util::human_size(item.done)
                    ),
                    Style::default().fg(Color::DarkGray),
                ));
                ListItem::new(vec![line1, line2])
            })
            .collect();
        frame.render_widget(List::new(items).block(block), area);
    }

    fn draw_file_select(&self, frame: &mut Frame, area: Rect) {
        let Some(select) = &self.file_select else {
            return;
        };
        let height = (select.members.len() as u16 + 3)
            .min(area.height.max(3))
            .max(3);
        let width = 56.min(area.width.saturating_sub(2)).max(20);
        let x = area.x + area.width.saturating_sub(width) / 2;
        let y = area.y + area.height.saturating_sub(height) / 2;
        let rect = Rect::new(x, y, width, height);
        frame.render_widget(Clear, rect);

        let items: Vec<ListItem> = select
            .members
            .iter()
            .enumerate()
            .map(|(index, member)| {
                let mark = if select.selected[index] { "[x]" } else { "[ ]" };
                let host = if member.host { " ★" } else { "" };
                let addr = member.addr.as_deref().unwrap_or("-");
                ListItem::new(vec![
                    Line::from(format!("{mark} {}{}", member.nick, host)),
                    Line::from(Span::styled(
                        format!("    {addr}"),
                        Style::default().fg(Color::DarkGray),
                    )),
                ])
            })
            .collect();
        let mut list_state = ListState::default().with_selected(Some(select.index));
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::Yellow))
            .title(Line::from(Span::styled(
                " 选择接收人 · 空格选择 · Enter 确认 · Esc 取消 ",
                Style::default().fg(Color::Yellow),
            )));
        let list = List::new(items)
            .block(block)
            .highlight_style(
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("▸ ");
        frame.render_stateful_widget(list, rect, &mut list_state);
    }

    fn draw_status(&self, frame: &mut Frame, area: Rect) {
        if self.elevate_core.is_some() {
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    "EasyTier 需要 TUN 权限：y 提权并记住 · o 仅本次 · n 不再提示",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ))),
                area,
            );
            return;
        }
        if let Some(select) = &self.file_select {
            let chosen = select.selected.iter().filter(|value| **value).count();
            let text = format!(
                "选择接收人：已选 {chosen} 人 · ↑/↓ 移动 · 空格选择 · Enter 确认 · Esc 取消"
            );
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    text,
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ))),
                area,
            );
            return;
        }

        if let Some(idx) = self.reply_select {
            if let Some(message) = self.messages.get(idx) {
                let text = format!(
                    "选择回复 #{} {}: {}  · ↑/↓ 选择 · Enter 确认 · Esc 取消",
                    message.seq,
                    message.nick,
                    truncate_width(&message.text, 50)
                );
                frame.render_widget(
                    Paragraph::new(Line::from(Span::styled(
                        text,
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    ))),
                    area,
                );
                return;
            }
        }
        let (text, color) = match &self.status {
            Some((text, at, error)) if at.elapsed() < Duration::from_secs(10) => (
                text.clone(),
                if *error { Color::Red } else { Color::Yellow },
            ),
            _ => {
                if self.offers.is_empty() {
                    ("就绪".to_string(), Color::DarkGray)
                } else {
                    (
                        format!(
                            "有 {} 个文件请求待处理：/accept <编号> 或 /deny <编号>",
                            self.offers.len()
                        ),
                        Color::Yellow,
                    )
                }
            }
        };
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(text, Style::default().fg(color)))),
            area,
        );
    }

    fn draw_reply_bar(&self, frame: &mut Frame, area: Rect) {
        let Some((seq, reply)) = &self.pending_reply else {
            return;
        };
        let head = format!("↩ 回复 #{seq} {}: ", truncate_width(&reply.nick, 12));
        let avail = (area.width as usize)
            .saturating_sub(UnicodeWidthStr::width(head.as_str()) + 12)
            .max(4);
        let text = format!(
            "{head}{}  (Esc 取消)",
            truncate_width(&reply.preview, avail)
        );
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                text,
                Style::default().fg(Color::Cyan),
            ))),
            area,
        );
    }

    /// Helix 风格补全弹窗：显示在输入行上方，选中项高亮。
    fn draw_completion_popup(&self, frame: &mut Frame, input_area: Rect) {
        let Some(state) = &self.completion else {
            return;
        };
        if state.candidates.is_empty() {
            return;
        }
        let max_width = state
            .candidates
            .iter()
            .map(|item| UnicodeWidthStr::width(item.as_str()))
            .max()
            .unwrap_or(12)
            .min(52) as u16;
        let width = (max_width + 4)
            .max(24)
            .min(frame.area().width.saturating_sub(4));
        let visible = state.candidates.len().min(9) as u16;
        let height = visible + 2;
        let x = input_area.x.saturating_add(2);
        let y = input_area.y.saturating_sub(height);
        if width < 12 || input_area.y < height {
            return;
        }
        let rect = Rect::new(x, y, width, height);
        frame.render_widget(Clear, rect);

        let visible_count = visible as usize;
        let start = if state.index >= visible_count {
            state.index + 1 - visible_count
        } else {
            0
        };
        let items: Vec<ListItem> = state
            .candidates
            .iter()
            .skip(start)
            .take(visible_count)
            .map(|candidate| {
                ListItem::new(Line::from(Span::styled(
                    candidate.clone(),
                    Style::default().fg(Color::White),
                )))
            })
            .collect();
        let mut list_state =
            ListState::default().with_selected(Some(state.index.saturating_sub(start)));
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::Yellow))
            .title(Line::from(Span::styled(
                format!(" 补全 {}/{} ", state.index + 1, state.candidates.len()),
                Style::default().fg(Color::Yellow),
            )));
        let list = List::new(items)
            .block(block)
            .highlight_style(
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("▸ ");
        frame.render_stateful_widget(list, rect, &mut list_state);
    }

    fn draw_input(&self, frame: &mut Frame, area: Rect) {
        let (visible, cursor_x) = self.input_view(area.width.saturating_sub(2) as usize);
        let mut spans = vec![Span::styled(
            "> ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )];
        if visible.is_empty() {
            spans.push(Span::styled(
                "输入消息，或 /help 查看命令",
                Style::default().fg(Color::DarkGray),
            ));
        } else {
            spans.push(Span::styled(visible, Style::default().fg(Color::White)));
        }
        frame.render_widget(Paragraph::new(Line::from(spans)), area);
        if !self.vim_mode || self.input_mode == InputMode::Insert {
            frame
                .set_cursor_position((area.x + cursor_x.min(area.width.saturating_sub(1)), area.y));
        }
    }

    fn draw_footer(&self, frame: &mut Frame, area: Rect) {
        let hints = if self.vim_mode {
            if self.input_mode == InputMode::Normal {
                "浏览 · i 输入 · j/k 滚动 · g/G 顶/底 · PgUp/PgDn · Ctrl+C 退出"
            } else {
                "输入 · Enter 发送 · Esc 浏览 · Tab 补全 · Ctrl+R 回复 · PgUp/PgDn 滚动 · Ctrl+C 退出"
            }
        } else {
            HINTS
        };
        let mut spans = vec![Span::styled(hints, Style::default().fg(Color::DarkGray))];
        if self.vim_mode {
            let mode = if self.input_mode == InputMode::Normal {
                "浏览"
            } else {
                "输入"
            };
            spans.push(Span::styled(
                format!("  [{mode}]"),
                Style::default().fg(Color::Cyan),
            ));
        }
        if let Some(incoming) = &self.incoming_call {
            spans.push(Span::styled(
                format!("  来电: {}", incoming.nick),
                Style::default().fg(Color::Yellow),
            ));
        }
        if let Some(call) = &self.active_call {
            spans.push(Span::styled(
                format!(
                    "  通话中: {}{}",
                    call.nick,
                    if call.muted { " (静音)" } else { "" }
                ),
                Style::default().fg(Color::Green),
            ));
        }
        if self.voice_room_joined {
            let count = self.voice_room_members.len();
            spans.push(Span::styled(
                format!(
                    "  语音房: {count} 人{}",
                    if self.voice_room_muted {
                        " (静音)"
                    } else {
                        ""
                    }
                ),
                Style::default().fg(Color::Magenta),
            ));
        }
        if self.scroll > 0 {
            let back = if self.vim_mode {
                "G 回到底部"
            } else {
                "End 回到底部"
            };
            spans.push(Span::styled(
                format!("  ↑ 已向上 {} 行 ({back})", self.scroll),
                Style::default().fg(Color::Yellow),
            ));
        }
        frame.render_widget(Paragraph::new(Line::from(spans)), area);
    }

    fn input_view(&self, width: usize) -> (String, u16) {
        if width == 0 {
            return (String::new(), 0);
        }
        let cursor_byte = byte_index(&self.input, self.cursor);
        let before: Vec<char> = self.input[..cursor_byte].chars().collect();
        let after: Vec<char> = self.input[cursor_byte..].chars().collect();
        let mut used = 0usize;
        let mut start = before.len();
        for (idx, ch) in before.iter().enumerate().rev() {
            let w = ch.width().unwrap_or(0);
            if used + w + 1 > width {
                break;
            }
            used += w;
            start = idx;
        }
        let mut shown: String = before[start..].iter().collect();
        let mut total_width = used;
        for ch in after {
            let w = ch.width().unwrap_or(0);
            if total_width + w > width {
                break;
            }
            total_width += w;
            shown.push(ch);
        }
        (shown, 2 + used as u16)
    }
}

/// 启动 TUI 主循环（阻塞当前线程）。
pub fn run(handle: NodeHandle, cfg: UiConfig) -> Result<TuiAction> {
    install_panic_hook();
    let mut terminal = init_terminal()?;
    let mut app = App::new(&handle, cfg);
    let mut mouse_capture = true;
    let events = handle.events;
    while !app.should_quit {
        while let Ok(event) = events.try_recv() {
            app.on_event(event);
        }
        app.pump_voice();
        app.pump_voice_room();
        if event::poll(Duration::from_millis(60))? {
            match event::read()? {
                Event::Key(key) => {
                    // Windows 会同时上报 Press 和 Release；只处理按下与长按重复，
                    // 否则每个字符会被输入两次。
                    if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
                        app.on_key(key);
                    }
                }
                Event::Paste(text) => app.insert_text(&text),
                Event::Mouse(mouse) => app.on_mouse(mouse),
                Event::Resize(_, _) => {}
                _ => {}
            }
        }
        terminal.draw(|frame| app.draw(frame))?;
        if app.vim_mode && app.input_mode == InputMode::Normal {
            let _ = terminal.hide_cursor();
        } else {
            let _ = terminal.show_cursor();
        }
        if app.mouse_capture != mouse_capture {
            mouse_capture = app.mouse_capture;
            if mouse_capture {
                crossterm::execute!(terminal.backend_mut(), EnableMouseCapture)?;
            } else {
                crossterm::execute!(terminal.backend_mut(), DisableMouseCapture)?;
            }
        }
        if app.action.is_some() {
            break;
        }
    }
    restore_terminal(&mut terminal)?;
    let _ = handle.cmd.send(Cmd::Quit);
    Ok(app.action.take().unwrap_or(TuiAction::Quit))
}

fn init_terminal() -> Result<Terminal<CrosstermBackend<std::io::Stdout>>> {
    terminal::enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    crossterm::execute!(
        stdout,
        terminal::EnterAlternateScreen,
        EnableBracketedPaste,
        EnableMouseCapture
    )?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;
    Ok(terminal)
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>) -> Result<()> {
    terminal::disable_raw_mode()?;
    crossterm::execute!(
        terminal.backend_mut(),
        terminal::LeaveAlternateScreen,
        DisableBracketedPaste,
        DisableMouseCapture,
        cursor::Show
    )?;
    terminal.show_cursor()?;
    Ok(())
}

fn install_panic_hook() {
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = terminal::disable_raw_mode();
        let _ = crossterm::execute!(
            std::io::stdout(),
            terminal::LeaveAlternateScreen,
            DisableBracketedPaste,
            DisableMouseCapture,
            cursor::Show
        );
        hook(info);
    }));
}

/// 计算主体区域划分：宽屏时聊天区在左，成员区/传输区在右侧上下分割；
/// 窄屏时只保留聊天区（成员/传输通过 /peers、/files 查看）。
fn body_layout(area: Rect, wide: bool) -> (Rect, Option<(Rect, Rect)>) {
    if !wide {
        return (area, None);
    }
    let cols = Layout::new(
        Direction::Horizontal,
        [Constraint::Percentage(74), Constraint::Percentage(26)],
    )
    .split(area);
    let rows = Layout::new(
        Direction::Vertical,
        [Constraint::Min(5), Constraint::Length(9)],
    )
    .split(cols[1]);
    (cols[0], Some((rows[0], rows[1])))
}

/// 简单的 `*` / `?` 通配符匹配（不区分路径层级）。
fn wildcard_match(pattern: &str, name: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let n: Vec<char> = name.chars().collect();
    let (mut pi, mut ni) = (0usize, 0usize);
    let (mut star, mut mark) = (None, 0usize);
    while ni < n.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == n[ni]) {
            pi += 1;
            ni += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ni;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ni = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

/// 展开通配符路径；只支持文件名/目录名中的 `*` 和 `?`。
fn expand_glob(pattern: &Path) -> Vec<PathBuf> {
    use std::path::Component;
    let mut current: Vec<PathBuf> = vec![PathBuf::new()];
    for component in pattern.components() {
        match component {
            Component::RootDir => current = vec![PathBuf::from("/")],
            Component::Prefix(prefix) => {
                current = vec![PathBuf::from(prefix.as_os_str())];
            }
            Component::CurDir => {}
            Component::ParentDir => {
                current = current.into_iter().map(|base| base.join("..")).collect();
            }
            Component::Normal(name) => {
                let name = name.to_string_lossy();
                if name.contains('*') || name.contains('?') {
                    let mut next = Vec::new();
                    for base in &current {
                        let dir = if base.as_os_str().is_empty() {
                            Path::new(".")
                        } else {
                            base.as_path()
                        };
                        let Ok(entries) = std::fs::read_dir(dir) else {
                            continue;
                        };
                        for entry in entries.flatten() {
                            let file_name = entry.file_name().to_string_lossy().to_string();
                            if file_name.starts_with('.') && !name.starts_with('.') {
                                continue;
                            }
                            if wildcard_match(&name, &file_name) {
                                next.push(base.join(file_name));
                            }
                        }
                    }
                    current = next;
                } else {
                    current = current
                        .into_iter()
                        .map(|base| base.join(name.as_ref()))
                        .collect();
                }
            }
        }
    }
    current
}

fn collect_send_files(path: &Path, out: &mut Vec<PathBuf>, limit: usize, depth: usize) {
    if out.len() >= limit || depth > 12 {
        return;
    }
    if path.is_file() {
        if !out.contains(&path.to_path_buf()) {
            out.push(path.to_path_buf());
        }
        return;
    }
    if let Ok(entries) = std::fs::read_dir(path) {
        let mut entries: Vec<_> = entries.flatten().collect();
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            collect_send_files(&entry.path(), out, limit, depth + 1);
            if out.len() >= limit {
                break;
            }
        }
    }
}

/// 把 `/send` 的路径参数展开成文件列表：支持目录递归和 `*`/`?` 通配符。
fn expand_send_files(raw: &str, limit: usize) -> Vec<PathBuf> {
    let expanded = crate::util::expand_tilde(raw);
    let pattern = PathBuf::from(expanded);
    let mut out = Vec::new();
    for candidate in expand_glob(&pattern) {
        collect_send_files(&candidate, &mut out, limit, 0);
        if out.len() >= limit {
            break;
        }
    }
    out
}

/// 文件/目录路径补全。返回替换当前 token 的候选值；目录以 `/` 结尾。
fn path_candidates(token: &str) -> Vec<String> {
    let home = crate::paths::home_dir().ok();
    let (display_dir, fs_dir, prefix) = match token.rfind(['/', '\\']) {
        Some(index) => {
            let display = &token[..=index];
            let fs = if let Some(home) = &home {
                if let Some(rest) = display.strip_prefix("~/") {
                    home.join(rest)
                } else if display == "~/" || display == "~\\" {
                    home.clone()
                } else {
                    std::path::PathBuf::from(display)
                }
            } else {
                std::path::PathBuf::from(display)
            };
            (display.to_string(), fs, token[index + 1..].to_string())
        }
        None => {
            if token == "~" {
                (
                    "~/".to_string(),
                    home.clone().unwrap_or_else(|| ".".into()),
                    String::new(),
                )
            } else {
                (
                    String::new(),
                    std::path::PathBuf::from("."),
                    token.to_string(),
                )
            }
        }
    };
    let Ok(read) = std::fs::read_dir(&fs_dir) else {
        return Vec::new();
    };
    let mut entries: Vec<(bool, String, String)> = Vec::new();
    for entry in read.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') && !prefix.starts_with('.') {
            continue;
        }
        if !name.to_lowercase().starts_with(&prefix.to_lowercase()) {
            continue;
        }
        let is_dir = entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false);
        let mut value = format!("{display_dir}{name}");
        if is_dir {
            value.push('/');
        }
        entries.push((is_dir, name, value));
    }
    entries.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    entries
        .into_iter()
        .take(50)
        .map(|(_, _, value)| value)
        .collect()
}

fn byte_index(text: &str, char_index: usize) -> usize {
    text.char_indices()
        .nth(char_index)
        .map(|(idx, _)| idx)
        .unwrap_or(text.len())
}

fn nick_color(nick: &str) -> Color {
    const COLORS: [Color; 7] = [
        Color::Green,
        Color::LightYellow,
        Color::LightMagenta,
        Color::LightBlue,
        Color::LightGreen,
        Color::Magenta,
        Color::Yellow,
    ];
    COLORS[util::color_index(nick) % COLORS.len()]
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// 按终端显示宽度截断文本（中文等宽字符按 2 列计）。
fn truncate_width(text: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    if UnicodeWidthStr::width(text) <= max {
        return text.to_string();
    }
    let mut out = String::new();
    let mut used = 0usize;
    for ch in text.chars() {
        let width = ch.width().unwrap_or(0);
        if used + width + 1 > max {
            break;
        }
        out.push(ch);
        used += width;
    }
    out.push('…');
    out
}

fn wrap_with_prefix(
    prefix: &str,
    prefix_style: Style,
    text: &str,
    text_style: Style,
    width: usize,
    out: &mut Vec<Line<'static>>,
) {
    let prefix_width = UnicodeWidthStr::width(prefix);
    let first_width = width.saturating_sub(prefix_width).max(6);
    let parts = wrap_into(text, first_width, width.max(6));
    for (idx, part) in parts.iter().enumerate() {
        let mut spans = Vec::new();
        if idx == 0 {
            spans.push(Span::styled(prefix.to_string(), prefix_style));
        } else {
            spans.push(Span::raw(" ".repeat(prefix_width.min(width))));
        }
        spans.push(Span::styled(part.clone(), text_style));
        out.push(Line::from(spans));
    }
}

fn wrap_into(text: &str, first_width: usize, rest_width: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut current_width = 0usize;
    let mut limit = first_width.max(1);
    for ch in text.chars() {
        if ch == '\n' {
            out.push(std::mem::take(&mut current));
            current_width = 0;
            limit = rest_width.max(1);
            continue;
        }
        let w = ch.width().unwrap_or(0);
        if current_width + w > limit && !current.is_empty() {
            out.push(std::mem::take(&mut current));
            current_width = 0;
            limit = rest_width.max(1);
        }
        current.push(ch);
        current_width += w;
    }
    if !current.is_empty() || out.is_empty() {
        out.push(current);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrapping_respects_width() {
        let parts = wrap_into("abcdefghij", 4, 4);
        assert_eq!(parts, vec!["abcd", "efgh", "ij"]);
    }

    #[test]
    fn wrapping_handles_wide_chars() {
        let parts = wrap_into("中文测试内容", 4, 4);
        assert_eq!(parts, vec!["中文", "测试", "内容"]);
    }

    #[test]
    fn byte_index_handles_unicode() {
        assert_eq!(byte_index("中文", 0), 0);
        assert_eq!(byte_index("中文", 1), 3);
        assert_eq!(byte_index("中文", 2), 6);
    }

    #[test]
    fn body_layout_puts_side_panel_on_the_right() {
        let area = Rect::new(0, 0, 130, 40);
        let (chat, side) = body_layout(area, true);
        let (members, transfers) = side.expect("宽屏应有侧栏");

        // 聊天区占据左侧，右侧栏在其右方，两者宽度之和等于整体宽度。
        assert_eq!(chat.x, area.x);
        assert_eq!(chat.y, area.y);
        assert_eq!(chat.height, area.height);
        assert_eq!(members.x, chat.right());
        assert_eq!(chat.width + members.width, area.width);

        // 成员区与传输区在右侧栏内上下排列，互不重叠。
        assert_eq!(members.y, area.y);
        assert_eq!(transfers.y, members.bottom());
        assert_eq!(transfers.height, 9);
        assert_eq!(transfers.bottom(), area.bottom());
        assert!(members.height >= 5);
    }

    #[test]
    fn body_layout_single_column_when_narrow() {
        let area = Rect::new(0, 0, 60, 20);
        let (chat, side) = body_layout(area, false);
        assert_eq!(chat, area);
        assert!(side.is_none());
    }

    fn test_app() -> App {
        let (cmd, _rx) = tokio::sync::mpsc::unbounded_channel();
        App {
            cmd,
            input: String::new(),
            cursor: 0,
            messages: Vec::new(),
            members: vec![
                MemberView {
                    pubkey: "pk-me".to_string(),
                    peer_id: "me".to_string(),
                    nick: "me".to_string(),
                    addr: None,
                    host: false,
                    me: true,
                    presence: 0,
                },
                MemberView {
                    pubkey: "pk-alice".to_string(),
                    peer_id: "alice".to_string(),
                    nick: "alice".to_string(),
                    addr: None,
                    host: false,
                    me: false,
                    presence: 0,
                },
                MemberView {
                    pubkey: "pk-bob".to_string(),
                    peer_id: "bob".to_string(),
                    nick: "bob".to_string(),
                    addr: None,
                    host: false,
                    me: false,
                    presence: 0,
                },
            ],
            host: None,
            me: "pk-me".to_string(),
            nick: "me".to_string(),
            peer_id: "me".to_string(),
            room_id: "room".to_string(),
            room_name: None,
            group_path: std::path::PathBuf::from("target/test-group.json"),
            groups: Vec::new(),
            action: None,
            easytier: Default::default(),
            config_nodes: Vec::new(),
            dirs: crate::paths::RuntimeDirs::ensure(std::path::Path::new("target/test-runtime"))
                .unwrap(),
            input_mode: InputMode::Insert,
            vim_mode: false,
            scroll_step: 3,
            voice: crate::config::VoicePrefs::default(),
            active_call: None,
            incoming_call: None,
            voice_message: None,
            ptt_active: false,
            voice_room_joined: false,
            voice_room_members: Vec::new(),
            voice_room_audio: None,
            voice_room_muted: false,
            voice_last_send: Instant::now(),
            mouse_capture: true,
            elevate_core: None,
            listen: "127.0.0.1:7777".parse().unwrap(),
            invite: String::new(),
            max_file: 0,
            transfers: Vec::new(),
            offers: Vec::new(),
            status: None,
            scroll: 0,
            follow: true,
            history: Vec::new(),
            hist_pos: None,
            should_quit: false,
            wide_layout: true,
            next_seq: 1,
            pending_reply: None,
            reply_select: None,
            completion: None,
            file_select: None,
            last_copy: None,
        }
    }

    fn type_text(app: &mut App, text: &str) {
        app.input = text.to_string();
        app.cursor = app.input.chars().count();
    }

    fn chat_message(text: &str, mentions: Vec<String>, reply: Option<ReplyRef>) -> Message {
        Message {
            seq: 0,
            ts: 0,
            nick: "alice".to_string(),
            text: text.to_string(),
            kind: Kind::Chat,
            mine: false,
            id: "mid".to_string(),
            mentions,
            reply,
        }
    }

    #[test]
    fn tab_completes_command_name() {
        let mut app = test_app();
        type_text(&mut app, "/he");
        app.complete(false);
        assert_eq!(app.input, "/help ");
        assert_eq!(app.cursor, 6);
    }

    #[test]
    fn tab_cycles_command_candidates() {
        let mut app = test_app();
        type_text(&mut app, "/");
        app.complete(false);
        assert_eq!(app.input, "/help ");
        app.complete(false);
        assert_eq!(app.input, "/name ");
        app.complete(true);
        assert_eq!(app.input, "/help ");
    }

    #[test]
    fn tab_completes_mentions_and_everyone() {
        let mut app = test_app();
        type_text(&mut app, "@");
        app.complete(false);
        assert_eq!(app.input, "@alice ");
        app.complete(false);
        assert_eq!(app.input, "@bob ");
        app.complete(false);
        assert_eq!(app.input, "@所有人 ");
    }

    #[test]
    fn tab_completes_send_target_member() {
        let mut app = test_app();
        type_text(&mut app, "/send ");
        app.complete(false);
        assert_eq!(app.input, "/send alice ");
    }

    #[test]
    fn reply_command_sets_pending_reference() {
        let mut app = test_app();
        app.push_system("系统消息");
        app.push_message(chat_message("@me 你好", vec!["pk-me".to_string()], None));
        app.command_reply("2");
        let (seq, reply) = app.pending_reply.as_ref().expect("应设置回复");
        assert_eq!(*seq, 2);
        assert_eq!(reply.id, "mid");
        assert_eq!(reply.preview, "@me 你好");

        // 再次用 /reply 覆盖为同一编号也应正常。
        app.command_reply("#2");
        assert!(app.pending_reply.is_some());

        let (lines, starts) = app.chat_lines(80);
        assert_eq!(starts.len(), 2);
        assert!(lines
            .iter()
            .any(|line| line.spans.iter().any(|span| span.content.contains("@ "))));
    }

    #[test]
    fn chat_lines_render_reply_quote() {
        let mut app = test_app();
        let reply = ReplyRef {
            id: "orig".to_string(),
            nick: "bob".to_string(),
            preview: "原始消息".to_string(),
        };
        app.push_message(chat_message("收到", Vec::new(), Some(reply)));
        let (lines, _) = app.chat_lines(80);
        assert!(lines.iter().any(|line| line
            .spans
            .iter()
            .any(|span| span.content.contains("│ bob: 原始消息"))));
    }

    #[test]
    fn truncate_width_counts_wide_chars() {
        assert_eq!(truncate_width("abcdef", 10), "abcdef");
        assert_eq!(truncate_width("中文测试", 4), "中…");
        assert_eq!(truncate_width("中文测试", 6), "中文…");
    }

    #[test]
    fn send_targets_expand_all_and_lists() {
        let app = test_app();
        let all = app.resolve_send_targets("@all");
        assert_eq!(all.len(), 2);
        assert!(all.contains(&"pk-alice".to_string()));
        assert!(all.contains(&"pk-bob".to_string()));
        let picked = app.resolve_send_targets("alice, @bob");
        assert_eq!(picked, vec!["alice".to_string(), "bob".to_string()]);
    }

    #[test]
    fn copy_last_puts_content_into_input() {
        let mut app = test_app();
        app.last_copy = Some("asa://example".to_string());
        app.copy_to_input();
        assert_eq!(app.input, "asa://example");
        assert_eq!(app.cursor, app.input.chars().count());
    }

    #[test]
    fn path_candidates_prefer_directories() {
        let dir = std::env::temp_dir().join(format!("asa-path-complete-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("beta")).unwrap();
        std::fs::write(dir.join("alpha.txt"), b"x").unwrap();
        let prefix = format!("{}/", dir.display());
        let values = path_candidates(&prefix);
        assert!(values.iter().any(|v| v.ends_with("beta/")));
        assert!(values.iter().any(|v| v.ends_with("alpha.txt")));
        assert_eq!(values.first().map(|v| v.ends_with('/')), Some(true));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn directory_completion_does_not_add_space() {
        let mut app = test_app();
        app.input = "~/Do".to_string();
        app.cursor = app.input.chars().count();
        app.completion = Some(Completion {
            start: 0,
            end: app.cursor,
            candidates: vec!["~/Documents/".to_string()],
            index: 0,
        });
        app.complete(false);
        assert_eq!(app.input, "~/Documents/");
        assert!(!app.input.ends_with(' '));
    }

    #[test]
    fn file_select_confirm_builds_multi_send_input() {
        let mut app = test_app();
        app.start_file_select();
        {
            let select = app.file_select.as_mut().expect("应进入多选");
            for flag in &mut select.selected {
                *flag = true;
            }
        }
        app.on_file_select_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(app.input.starts_with("/send "));
        assert!(app.input.contains("alice"));
        assert!(app.input.contains("bob"));
        assert!(app.file_select.is_none());
    }

    #[test]
    fn path_candidates_are_case_insensitive() {
        let dir = std::env::temp_dir().join(format!("asa-path-case-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Alpha.TXT"), b"x").unwrap();
        let prefix = format!("{}/alpha", dir.display());
        let values = path_candidates(&prefix);
        assert!(values.iter().any(|value| value.ends_with("Alpha.TXT")));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

// ---------------------------------------------------------------- 启动大厅

use crate::config::{self, GroupProfile};
use crate::identity::Identity;
use crate::paths::RuntimeDirs;
use crate::room::RoomConfig;
use crate::rooms;

#[derive(Clone, Copy, PartialEq, Eq)]
enum LobbyMode {
    List,
    Join,
    Rename,
    AddNode,
    CreatePassword,
    SetPassword,
    LanList,
    LanPassword,
    ConfirmDelete,
}

enum LobbyOutcome {
    Selected(RoomConfig),
    Quit,
}

struct LobbyApp {
    dirs: RuntimeDirs,
    identity: Identity,
    nick: String,
    groups: Vec<rooms::SavedGroup>,
    index: usize,
    mode: LobbyMode,
    input: String,
    cursor: usize,
    status: Option<(String, bool)>,
    config: config::Config,
    et_enabled: std::collections::HashMap<String, bool>,
    browser: Option<ChannelBrowser>,
    lan_rx: Option<std::sync::mpsc::Receiver<DiscoveredChannel>>,
    lan: Vec<DiscoveredChannel>,
    lan_index: usize,
    pending_channel: Option<DiscoveredChannel>,
    pending_room: Option<RoomConfig>,
    auto_selected: bool,
}

impl LobbyApp {
    fn new(dirs: RuntimeDirs, identity: Identity, nick: String) -> Result<Self> {
        let groups = rooms::list(&dirs)?;
        let index = groups.iter().position(|group| group.active).unwrap_or(0);
        let config = config::Config::ensure(&dirs)?;
        let (browser, lan_rx) = match ChannelBrowser::start() {
            Ok((browser, rx)) => (Some(browser), Some(rx)),
            Err(_) => (None, None),
        };
        let et_enabled = groups
            .iter()
            .map(|group| {
                let profile = GroupProfile::load_at(&crate::config::group_path(&dirs, &group.id))
                    .unwrap_or_default();
                (group.id.clone(), profile.easytier_enabled.unwrap_or(false))
            })
            .collect();
        Ok(Self {
            dirs,
            identity,
            nick,
            groups,
            index,
            mode: LobbyMode::List,
            input: String::new(),
            cursor: 0,
            status: None,
            config,
            et_enabled,
            browser,
            lan_rx,
            lan: Vec::new(),
            lan_index: 0,
            pending_channel: None,
            pending_room: None,
            auto_selected: false,
        })
    }

    fn refresh(&mut self) -> Result<()> {
        self.groups = rooms::list(&self.dirs)?;
        if self.index >= self.groups.len() {
            self.index = self.groups.len().saturating_sub(1);
        }
        self.et_enabled = self
            .groups
            .iter()
            .map(|group| {
                let profile =
                    GroupProfile::load_at(&crate::config::group_path(&self.dirs, &group.id))
                        .unwrap_or_default();
                (group.id.clone(), profile.easytier_enabled.unwrap_or(false))
            })
            .collect();
        Ok(())
    }

    fn set_status(&mut self, text: impl Into<String>, error: bool) {
        self.status = Some((text.into(), error));
    }

    fn drain_lan(&mut self) {
        if let Some(rx) = &self.lan_rx {
            while let Ok(channel) = rx.try_recv() {
                if !self
                    .lan
                    .iter()
                    .any(|item| item.room == channel.room && item.join_addr == channel.join_addr)
                {
                    self.lan.push(channel);
                }
            }
        }
        if self.auto_selected || self.mode != LobbyMode::List {
            return;
        }
        if let Some(pos) = self
            .groups
            .iter()
            .position(|group| self.lan.iter().any(|channel| channel.room == group.id))
        {
            let path = crate::config::group_path(&self.dirs, &self.groups[pos].id);
            let profile = GroupProfile::load_at(&path).unwrap_or_default();
            if profile.join_password_enc.is_some() {
                self.index = pos;
                self.auto_selected = true;
                let name = self.groups[pos].name.clone();
                self.set_status(format!("已发现「{name}」，按 Enter 自动认证"), false);
            }
        }
    }

    fn finish_create(&mut self, password: Option<&str>) -> Option<LobbyOutcome> {
        let room = self.pending_room.take()?;
        let path = crate::config::group_path(&self.dirs, &room.id);
        let mut profile = GroupProfile::load_at(&path).unwrap_or_default();
        profile.join_password = match password.filter(|value| !value.is_empty()) {
            Some(value) => match crate::config::JoinPassword::new(value) {
                Ok(password) => Some(password),
                Err(err) => {
                    self.set_status(format!("设置加入密码失败: {err}"), true);
                    self.pending_room = Some(room);
                    return None;
                }
            },
            None => None,
        };
        if let Err(err) = profile.save_at(&path) {
            self.set_status(format!("保存频道失败: {err}"), true);
            self.pending_room = Some(room);
            return None;
        }
        let _ = rooms::set_active(&self.dirs, &room.id);
        Some(LobbyOutcome::Selected(room))
    }

    fn set_join_password(&mut self, password: Option<&str>) {
        let Some(group) = self.groups.get(self.index).cloned() else {
            self.mode = LobbyMode::List;
            return;
        };
        let path = crate::config::group_path(&self.dirs, &group.id);
        let mut profile = GroupProfile::load_at(&path).unwrap_or_default();
        profile.join_password = match password.filter(|value| !value.is_empty()) {
            Some(value) => match crate::config::JoinPassword::new(value) {
                Ok(password) => Some(password),
                Err(err) => {
                    self.set_status(format!("设置加入密码失败: {err}"), true);
                    return;
                }
            },
            None => None,
        };
        match profile.save_at(&path) {
            Ok(()) => {
                self.mode = LobbyMode::List;
                self.input.clear();
                self.cursor = 0;
                self.set_status(
                    if profile.join_password.is_some() {
                        "频道加入密码已设置"
                    } else {
                        "频道加入密码已清除"
                    },
                    false,
                );
            }
            Err(err) => self.set_status(format!("保存密码失败: {err}"), true),
        }
    }

    fn join_lan(
        &mut self,
        channel: DiscoveredChannel,
        password: Option<&str>,
    ) -> Option<LobbyOutcome> {
        match crate::lan::request_invite(&channel, password) {
            Ok(invite) => match crate::lan::save_joined_channel(&self.dirs, &invite, password) {
                Ok(room) => Some(LobbyOutcome::Selected(room)),
                Err(err) => {
                    self.set_status(format!("保存频道失败: {err}"), true);
                    None
                }
            },
            Err(err) => {
                self.set_status(format!("加入失败: {err}"), true);
                None
            }
        }
    }

    fn insert_text(&mut self, text: &str) {
        let cleaned: String = text.chars().filter(|ch| !ch.is_control()).collect();
        if cleaned.is_empty() {
            return;
        }
        let byte = byte_index(&self.input, self.cursor);
        self.input.insert_str(byte, &cleaned);
        self.cursor += cleaned.chars().count();
    }

    fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let end = byte_index(&self.input, self.cursor);
        let start = byte_index(&self.input, self.cursor - 1);
        self.input.replace_range(start..end, "");
        self.cursor -= 1;
    }

    fn on_key(&mut self, key: KeyEvent) -> Option<LobbyOutcome> {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match self.mode {
            LobbyMode::List => match key.code {
                KeyCode::Char('q') if !ctrl => return Some(LobbyOutcome::Quit),
                KeyCode::Esc => return Some(LobbyOutcome::Quit),
                KeyCode::Up => {
                    if self.index > 0 {
                        self.index -= 1;
                    }
                }
                KeyCode::Down => {
                    if self.index + 1 < self.groups.len() {
                        self.index += 1;
                    }
                }
                KeyCode::Enter => {
                    let Some(group) = self.groups.get(self.index).cloned() else {
                        self.set_status("还没有频道，按 c 创建或 j 加入", true);
                        return None;
                    };
                    if let Some(channel) = self
                        .lan
                        .iter()
                        .find(|channel| channel.room == group.id)
                        .cloned()
                    {
                        let path = crate::config::group_path(&self.dirs, &group.id);
                        let profile = GroupProfile::load_at(&path).unwrap_or_default();
                        if let (Some(encoded), Ok(Some(room))) = (
                            profile.join_password_enc.as_deref(),
                            rooms::load(&self.dirs, &group.id),
                        ) {
                            if let Ok(password) =
                                crate::lan::open_saved_password(&room.key, encoded)
                            {
                                match crate::lan::request_invite(&channel, Some(&password)) {
                                    Ok(invite) => {
                                        match crate::lan::save_joined_channel(
                                            &self.dirs,
                                            &invite,
                                            Some(&password),
                                        ) {
                                            Ok(room) => {
                                                return Some(LobbyOutcome::Selected(room));
                                            }
                                            Err(err) => self.set_status(
                                                format!("自动认证后保存失败: {err}"),
                                                true,
                                            ),
                                        }
                                    }
                                    Err(err) => self.set_status(
                                        format!("自动认证失败，使用本地记录进入: {err}"),
                                        true,
                                    ),
                                }
                            }
                        }
                    }
                    match rooms::load(&self.dirs, &group.id) {
                        Ok(Some(room)) => {
                            let _ = rooms::set_active(&self.dirs, &room.id);
                            return Some(LobbyOutcome::Selected(room));
                        }
                        Ok(None) => self.set_status("频道配置缺失", true),
                        Err(err) => self.set_status(format!("读取频道失败: {err}"), true),
                    }
                }
                KeyCode::Char('c') if !ctrl => {
                    let room = RoomConfig::create(&self.identity);
                    match rooms::save(&self.dirs, &room) {
                        Ok(()) => {
                            let mut profile =
                                GroupProfile::load(&self.dirs, &room.id).unwrap_or_default();
                            if profile.nick.is_none() {
                                profile.nick = Some(self.nick.clone());
                            }
                            if profile.easytier_enabled.is_none() {
                                profile.easytier_enabled = Some(
                                    self.config.easytier.auto_enable
                                        && !self.config.easytier.nodes.is_empty(),
                                );
                            }
                            let _ = profile.save(&self.dirs, &room.id);
                            let _ = rooms::set_active(&self.dirs, &room.id);
                            self.pending_room = Some(room);
                            self.mode = LobbyMode::CreatePassword;
                            self.input.clear();
                            self.cursor = 0;
                        }
                        Err(err) => self.set_status(format!("创建频道失败: {err}"), true),
                    }
                }
                KeyCode::Char('j') if !ctrl => {
                    self.mode = LobbyMode::Join;
                    self.input.clear();
                    self.cursor = 0;
                }
                KeyCode::Char('r') if !ctrl => {
                    if let Some(group) = self.groups.get(self.index) {
                        self.mode = LobbyMode::Rename;
                        self.input = group.name.clone();
                        self.cursor = self.input.chars().count();
                    }
                }
                KeyCode::Char('d') if !ctrl => {
                    if self.groups.get(self.index).is_some() {
                        self.mode = LobbyMode::ConfirmDelete;
                    }
                }
                KeyCode::Char('n') if !ctrl => {
                    self.mode = LobbyMode::AddNode;
                    self.input.clear();
                    self.cursor = 0;
                }
                KeyCode::Char('e') if !ctrl => {
                    let Some(group) = self.groups.get(self.index).cloned() else {
                        self.set_status("先选择一个频道", true);
                        return None;
                    };
                    if self.config.easytier.nodes.is_empty() {
                        self.set_status("还没有 EasyTier 节点，按 n 添加", true);
                        return None;
                    }
                    let path = config::group_path(&self.dirs, &group.id);
                    let mut profile = GroupProfile::load_at(&path).unwrap_or_default();
                    let enabled = !profile.easytier_enabled.unwrap_or(false);
                    profile.easytier_enabled = Some(enabled);
                    if let Err(err) = profile.save_at(&path) {
                        self.set_status(format!("保存 EasyTier 开关失败: {err}"), true);
                        return None;
                    }
                    self.et_enabled.insert(group.id.clone(), enabled);
                    self.set_status(
                        format!(
                            "已{}「{}」的 EasyTier 内网穿透（进入频道后生效）",
                            if enabled { "开启" } else { "关闭" },
                            group.name
                        ),
                        false,
                    );
                }
                KeyCode::Char('f') if !ctrl => {
                    if self.browser.is_some() {
                        self.drain_lan();
                        self.mode = LobbyMode::LanList;
                        self.lan_index = 0;
                    } else {
                        self.set_status("局域网发现不可用", true);
                    }
                }
                KeyCode::Char('p') if !ctrl => {
                    if self.groups.get(self.index).is_some() {
                        self.mode = LobbyMode::SetPassword;
                        self.input.clear();
                        self.cursor = 0;
                    }
                }
                _ => {}
            },
            LobbyMode::Join => match key.code {
                KeyCode::Esc => {
                    self.mode = LobbyMode::List;
                    self.input.clear();
                    self.cursor = 0;
                }
                KeyCode::Enter => {
                    let text = self.input.trim().to_string();
                    if text.is_empty() {
                        self.set_status("邀请码不能为空", true);
                        return None;
                    }
                    match RoomConfig::parse_invite_full(&text) {
                        Ok((parsed, _addresses)) => match rooms::load(&self.dirs, &parsed.id) {
                            Ok(Some(mut existing)) => {
                                for entry in &parsed.entries {
                                    if !existing.entries.iter().any(|item| item == entry) {
                                        existing.entries.push(entry.clone());
                                    }
                                }
                                for relay in &parsed.relays {
                                    if !existing.relays.iter().any(|item| item == relay) {
                                        existing.relays.push(relay.clone());
                                    }
                                }
                                for node in &parsed.et_nodes {
                                    if !existing.et_nodes.iter().any(|item| item == node) {
                                        existing.et_nodes.push(node.clone());
                                    }
                                }
                                let mut config = self.config.clone();
                                let mut pool =
                                    crate::easytier::normalize_nodes(config.easytier.nodes.iter());
                                if crate::easytier::merge_pool(&mut pool, &parsed.et_nodes) {
                                    config.easytier.nodes = pool;
                                    let _ = config.save(&config::config_path(&self.dirs));
                                    self.config.easytier.nodes = config.easytier.nodes.clone();
                                }
                                match rooms::save(&self.dirs, &existing) {
                                    Ok(()) => {
                                        if !parsed.et_nodes.is_empty() {
                                            let path = config::group_path(&self.dirs, &existing.id);
                                            let mut profile =
                                                GroupProfile::load_at(&path).unwrap_or_default();
                                            if profile.easytier_enabled.is_none() {
                                                profile.easytier_enabled = Some(true);
                                                let _ = profile.save_at(&path);
                                            }
                                        }
                                        return Some(LobbyOutcome::Selected(existing));
                                    }
                                    Err(err) => {
                                        self.set_status(format!("保存频道失败: {err}"), true)
                                    }
                                }
                            }
                            Ok(None) => match rooms::save(&self.dirs, &parsed) {
                                Ok(()) => {
                                    let mut profile = GroupProfile::load(&self.dirs, &parsed.id)
                                        .unwrap_or_default();
                                    if profile.nick.is_none() {
                                        profile.nick = Some(self.nick.clone());
                                    }
                                    if !parsed.et_nodes.is_empty()
                                        && profile.easytier_enabled.is_none()
                                    {
                                        profile.easytier_enabled = Some(true);
                                    }
                                    let _ = profile.save(&self.dirs, &parsed.id);
                                    let _ = rooms::set_active(&self.dirs, &parsed.id);
                                    return Some(LobbyOutcome::Selected(parsed));
                                }
                                Err(err) => self.set_status(format!("保存频道失败: {err}"), true),
                            },
                            Err(err) => self.set_status(format!("读取频道失败: {err}"), true),
                        },
                        Err(err) => self.set_status(format!("邀请码无效: {err}"), true),
                    }
                }
                KeyCode::Backspace => self.backspace(),
                KeyCode::Left => {
                    if self.cursor > 0 {
                        self.cursor -= 1;
                    }
                }
                KeyCode::Right => {
                    if self.cursor < self.input.chars().count() {
                        self.cursor += 1;
                    }
                }
                KeyCode::Char(ch) if !ctrl => self.insert_text(&ch.to_string()),
                _ => {}
            },
            LobbyMode::Rename => match key.code {
                KeyCode::Esc => {
                    self.mode = LobbyMode::List;
                    self.input.clear();
                    self.cursor = 0;
                }
                KeyCode::Enter => {
                    let name: String = crate::util::sanitize_text(&self.input)
                        .chars()
                        .take(40)
                        .collect();
                    if name.is_empty() {
                        self.set_status("频道名称不能为空", true);
                        return None;
                    }
                    let Some(group) = self.groups.get(self.index) else {
                        self.mode = LobbyMode::List;
                        return None;
                    };
                    let path = config::group_path(&self.dirs, &group.id);
                    let mut profile = GroupProfile::load_at(&path).unwrap_or_default();
                    profile.name = Some(name.clone());
                    match profile.save_at(&path) {
                        Ok(()) => {
                            self.mode = LobbyMode::List;
                            let _ = self.refresh();
                            self.set_status("频道名称已更新", false);
                        }
                        Err(err) => self.set_status(format!("保存名称失败: {err}"), true),
                    }
                }
                KeyCode::Backspace => self.backspace(),
                KeyCode::Left => {
                    if self.cursor > 0 {
                        self.cursor -= 1;
                    }
                }
                KeyCode::Right => {
                    if self.cursor < self.input.chars().count() {
                        self.cursor += 1;
                    }
                }
                KeyCode::Char(ch) if !ctrl => self.insert_text(&ch.to_string()),
                _ => {}
            },
            LobbyMode::AddNode => match key.code {
                KeyCode::Esc => {
                    self.mode = LobbyMode::List;
                    self.input.clear();
                    self.cursor = 0;
                }
                KeyCode::Enter => {
                    let url = match crate::easytier::normalize_node(&self.input) {
                        Ok(url) => url,
                        Err(err) => {
                            self.set_status(format!("节点地址无效: {err}"), true);
                            return None;
                        }
                    };
                    if !self.config.easytier.nodes.contains(&url) {
                        self.config.easytier.nodes.push(url.clone());
                        let path = config::config_path(&self.dirs);
                        if let Err(err) = self.config.save(&path) {
                            self.set_status(format!("保存节点失败: {err}"), true);
                            return None;
                        }
                    }
                    if let Some(group) = self.groups.get(self.index).cloned() {
                        let path = config::group_path(&self.dirs, &group.id);
                        let mut profile = config::GroupProfile::load_at(&path).unwrap_or_default();
                        if profile.et_node.is_none() {
                            profile.et_node = Some(url.clone());
                            let _ = profile.save_at(&path);
                        }
                    }
                    self.mode = LobbyMode::List;
                    self.input.clear();
                    self.cursor = 0;
                    self.set_status(format!("已添加 EasyTier 节点: {url}"), false);
                }
                KeyCode::Backspace => self.backspace(),
                KeyCode::Left => {
                    if self.cursor > 0 {
                        self.cursor -= 1;
                    }
                }
                KeyCode::Right => {
                    if self.cursor < self.input.chars().count() {
                        self.cursor += 1;
                    }
                }
                KeyCode::Char(ch) if !ctrl => self.insert_text(&ch.to_string()),
                _ => {}
            },
            LobbyMode::CreatePassword => match key.code {
                KeyCode::Esc => {
                    if let Some(outcome) = self.finish_create(None) {
                        return Some(outcome);
                    }
                }
                KeyCode::Enter => {
                    let password = self.input.clone();
                    if let Some(outcome) = self.finish_create(Some(&password)) {
                        return Some(outcome);
                    }
                }
                KeyCode::Backspace => self.backspace(),
                KeyCode::Left => {
                    if self.cursor > 0 {
                        self.cursor -= 1;
                    }
                }
                KeyCode::Right => {
                    if self.cursor < self.input.chars().count() {
                        self.cursor += 1;
                    }
                }
                KeyCode::Char(ch) if !ctrl => self.insert_text(&ch.to_string()),
                _ => {}
            },
            LobbyMode::SetPassword => match key.code {
                KeyCode::Esc => {
                    self.mode = LobbyMode::List;
                    self.input.clear();
                    self.cursor = 0;
                }
                KeyCode::Enter => {
                    let password = self.input.clone();
                    self.set_join_password(Some(&password));
                }
                KeyCode::Backspace => self.backspace(),
                KeyCode::Left => {
                    if self.cursor > 0 {
                        self.cursor -= 1;
                    }
                }
                KeyCode::Right => {
                    if self.cursor < self.input.chars().count() {
                        self.cursor += 1;
                    }
                }
                KeyCode::Char(ch) if !ctrl => self.insert_text(&ch.to_string()),
                _ => {}
            },
            LobbyMode::LanList => match key.code {
                KeyCode::Esc => {
                    self.mode = LobbyMode::List;
                    self.lan_index = 0;
                }
                KeyCode::Up => {
                    if self.lan_index > 0 {
                        self.lan_index -= 1;
                    }
                }
                KeyCode::Down => {
                    if self.lan_index + 1 < self.lan.len() {
                        self.lan_index += 1;
                    }
                }
                KeyCode::Enter => {
                    let Some(channel) = self.lan.get(self.lan_index).cloned() else {
                        return None;
                    };
                    if channel.open {
                        return self.join_lan(channel, None);
                    }
                    self.pending_channel = Some(channel);
                    self.mode = LobbyMode::LanPassword;
                    self.input.clear();
                    self.cursor = 0;
                }
                _ => {}
            },
            LobbyMode::LanPassword => match key.code {
                KeyCode::Esc => {
                    self.mode = LobbyMode::LanList;
                    self.input.clear();
                    self.cursor = 0;
                }
                KeyCode::Enter => {
                    let Some(channel) = self.pending_channel.clone() else {
                        self.mode = LobbyMode::LanList;
                        return None;
                    };
                    let password = self.input.clone();
                    if let Some(outcome) = self.join_lan(channel, Some(&password)) {
                        return Some(outcome);
                    }
                }
                KeyCode::Backspace => self.backspace(),
                KeyCode::Left => {
                    if self.cursor > 0 {
                        self.cursor -= 1;
                    }
                }
                KeyCode::Right => {
                    if self.cursor < self.input.chars().count() {
                        self.cursor += 1;
                    }
                }
                KeyCode::Char(ch) if !ctrl => self.insert_text(&ch.to_string()),
                _ => {}
            },
            LobbyMode::ConfirmDelete => match key.code {
                KeyCode::Esc => self.mode = LobbyMode::List,
                KeyCode::Enter | KeyCode::Char('y') => {
                    if let Some(group) = self.groups.get(self.index).cloned() {
                        match rooms::remove(&self.dirs, &group.id) {
                            Ok(()) => {
                                self.mode = LobbyMode::List;
                                let _ = self.refresh();
                                self.set_status("频道已从本机删除", false);
                            }
                            Err(err) => self.set_status(format!("删除失败: {err}"), true),
                        }
                    }
                }
                _ => {}
            },
        }
        None
    }

    fn draw(&self, frame: &mut Frame) {
        let area = frame.area();
        frame.render_widget(Clear, area);
        frame.render_widget(
            Block::default().style(Style::default().bg(Color::Black)),
            area,
        );
        let width = 72.min(area.width.saturating_sub(4)).max(24);
        let content_len = if self.mode == LobbyMode::LanList {
            self.lan.len()
        } else {
            self.groups.len()
        };
        let height = (content_len as u16 + 12)
            .min(area.height.saturating_sub(2))
            .max(12);
        let x = area.x + area.width.saturating_sub(width) / 2;
        let y = area.y + area.height.saturating_sub(height) / 2;
        let rect = Rect::new(x, y, width, height);

        let mut lines: Vec<Line<'static>> = Vec::new();
        if self.mode == LobbyMode::LanList {
            if self.lan.is_empty() {
                lines.push(Line::from(Span::styled(
                    "正在搜索局域网频道…（需要宿主已进入频道）",
                    Style::default().fg(Color::Gray),
                )));
            } else {
                for (index, channel) in self.lan.iter().enumerate() {
                    let selected = index == self.lan_index;
                    let marker = if selected { "> " } else { "  " };
                    let access = if channel.open { "开放" } else { "密码" };
                    let style = if selected {
                        Style::default()
                            .fg(Color::Black)
                            .bg(Color::Yellow)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::White)
                    };
                    lines.push(Line::from(Span::styled(
                        format!(
                            "{marker}{:<24} {:<16} {access} {}",
                            truncate_width(&channel.name, 24),
                            truncate_width(&channel.host, 16),
                            &channel.room[..channel.room.len().min(8)]
                        ),
                        style,
                    )));
                }
            }
        } else if self.groups.is_empty() {
            lines.push(Line::from(Span::styled(
                "还没有保存的频道。",
                Style::default().fg(Color::Gray),
            )));
        } else {
            for (index, group) in self.groups.iter().enumerate() {
                let selected = index == self.index;
                let marker = if selected { "> " } else { "  " };
                let active = if group.active { "● " } else { "  " };
                let et_mark = if self.et_enabled.get(&group.id).copied().unwrap_or(false) {
                    "⇄ "
                } else {
                    "  "
                };
                let lan_mark = if self.lan.iter().any(|channel| channel.room == group.id) {
                    "⌁ "
                } else {
                    "  "
                };
                let style = if selected {
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Yellow)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::White)
                };
                let id = &group.id[..group.id.len().min(8)];
                lines.push(Line::from(Span::styled(
                    format!(
                        "{marker}{active}{et_mark}{lan_mark}{:<30} {id}",
                        truncate_width(&group.name, 30)
                    ),
                    style,
                )));
            }
        }

        lines.push(Line::from(""));
        if self.config.easytier.nodes.is_empty() {
            lines.push(Line::from(Span::styled(
                "未配置 EasyTier 节点（局域网仍可用），按 n 添加节点",
                Style::default().fg(Color::DarkGray),
            )));
        }
        if let Some((text, error)) = &self.status {
            lines.push(Line::from(Span::styled(
                text.clone(),
                Style::default().fg(if *error { Color::Red } else { Color::Yellow }),
            )));
        }
        match self.mode {
            LobbyMode::List => {
                lines.push(Line::from(Span::styled(
                    format!(
                        "↑/↓ 选择 · Enter 进入 · c 创建 · j 加入 · f 局域网 · r 重命名 · d 删除 · n 节点 · e 内网穿透 · p 加入密码 · 节点 {} · q 退出",
                        self.config.easytier.nodes.len()
                    ),
                    Style::default().fg(Color::DarkGray),
                )));
            }
            LobbyMode::Join => {
                lines.push(Line::from(Span::styled(
                    "粘贴邀请码后回车加入（Esc 返回）",
                    Style::default().fg(Color::Cyan),
                )));
                lines.push(Line::from(Span::styled(
                    format!("> {}", truncate_width(&self.input, width as usize - 6)),
                    Style::default().fg(Color::White),
                )));
            }
            LobbyMode::Rename => {
                lines.push(Line::from(Span::styled(
                    "输入新的频道名称后回车（Esc 返回）",
                    Style::default().fg(Color::Cyan),
                )));
                lines.push(Line::from(Span::styled(
                    format!("> {}", truncate_width(&self.input, width as usize - 6)),
                    Style::default().fg(Color::White),
                )));
            }
            LobbyMode::AddNode => {
                lines.push(Line::from(Span::styled(
                    "粘贴 EasyTier 节点地址后回车，例如 tcp://host:11010（Esc 返回）",
                    Style::default().fg(Color::Cyan),
                )));
                lines.push(Line::from(Span::styled(
                    format!("> {}", truncate_width(&self.input, width as usize - 6)),
                    Style::default().fg(Color::White),
                )));
            }
            LobbyMode::CreatePassword => {
                lines.push(Line::from(Span::styled(
                    "设置频道加入密码（直接回车跳过，Esc 跳过）",
                    Style::default().fg(Color::Cyan),
                )));
                lines.push(Line::from(Span::styled(
                    format!("> {}", "*".repeat(self.input.chars().count())),
                    Style::default().fg(Color::White),
                )));
            }
            LobbyMode::SetPassword => {
                lines.push(Line::from(Span::styled(
                    "输入新的加入密码（直接回车清除，Esc 返回）",
                    Style::default().fg(Color::Cyan),
                )));
                lines.push(Line::from(Span::styled(
                    format!("> {}", "*".repeat(self.input.chars().count())),
                    Style::default().fg(Color::White),
                )));
            }
            LobbyMode::LanList => {
                lines.push(Line::from(Span::styled(
                    "↑/↓ 选择 · Enter 加入 · Esc 返回 · 仅显示局域网内已开启的频道",
                    Style::default().fg(Color::DarkGray),
                )));
            }
            LobbyMode::LanPassword => {
                lines.push(Line::from(Span::styled(
                    "输入频道加入密码后回车（Esc 返回）",
                    Style::default().fg(Color::Cyan),
                )));
                lines.push(Line::from(Span::styled(
                    format!("> {}", "*".repeat(self.input.chars().count())),
                    Style::default().fg(Color::White),
                )));
            }
            LobbyMode::ConfirmDelete => {
                let name = self
                    .groups
                    .get(self.index)
                    .map(|group| group.name.clone())
                    .unwrap_or_default();
                lines.push(Line::from(Span::styled(
                    format!("删除频道「{name}」？Enter/y 确认 · Esc 取消"),
                    Style::default().fg(Color::Red),
                )));
            }
        }

        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::Cyan))
            .style(Style::default().bg(Color::Black))
            .title(Line::from(Span::styled(
                " ASA · 选择频道 ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )));
        frame.render_widget(Paragraph::new(lines).block(block), rect);

        if matches!(
            self.mode,
            LobbyMode::Join
                | LobbyMode::Rename
                | LobbyMode::AddNode
                | LobbyMode::CreatePassword
                | LobbyMode::SetPassword
                | LobbyMode::LanPassword
        ) {
            let status_line = u16::from(self.status.is_some());
            let cursor_y = if matches!(
                self.mode,
                LobbyMode::CreatePassword | LobbyMode::SetPassword | LobbyMode::LanPassword
            ) {
                y + 1 + status_line + 1
            } else {
                y + 1 + self.groups.len() as u16 + 2 + status_line
            };
            let cursor_x = x + 3 + self.cursor as u16;
            if cursor_y < area.bottom() && cursor_x < area.right() {
                frame.set_cursor_position((cursor_x, cursor_y));
            }
        }
    }
}

/// 启动时的不透明选择框：选择已保存频道，或创建/加入/重命名/删除。
pub fn lobby(dirs: RuntimeDirs, identity: Identity, nick: String) -> Result<Option<RoomConfig>> {
    install_panic_hook();
    let mut terminal = init_terminal()?;
    let mut app = LobbyApp::new(dirs, identity, nick)?;
    loop {
        app.drain_lan();
        terminal.draw(|frame| app.draw(frame))?;
        if event::poll(Duration::from_millis(80))? {
            match event::read()? {
                Event::Key(key) => {
                    if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
                        match app.on_key(key) {
                            Some(LobbyOutcome::Selected(room)) => {
                                restore_terminal(&mut terminal)?;
                                return Ok(Some(room));
                            }
                            Some(LobbyOutcome::Quit) => {
                                restore_terminal(&mut terminal)?;
                                return Ok(None);
                            }
                            None => {}
                        }
                    }
                }
                Event::Paste(text) => {
                    if matches!(
                        app.mode,
                        LobbyMode::Join
                            | LobbyMode::Rename
                            | LobbyMode::AddNode
                            | LobbyMode::CreatePassword
                            | LobbyMode::SetPassword
                            | LobbyMode::LanPassword
                    ) {
                        app.insert_text(&text);
                    }
                }
                Event::Resize(_, _) => {}
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod send_tests {
    use super::*;

    #[test]
    fn wildcard_and_folder_expansion() {
        let root = std::env::temp_dir().join(format!("asa-send-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("a.txt"), b"a").unwrap();
        std::fs::write(root.join("b.txt"), b"b").unwrap();
        std::fs::write(root.join("sub").join("c.dat"), b"c").unwrap();

        let txt = expand_send_files(&format!("{}/*.txt", root.display()), 100);
        assert_eq!(txt.len(), 2, "{txt:?}");

        let all = expand_send_files(&root.display().to_string(), 100);
        assert_eq!(all.len(), 3, "{all:?}");

        let dat = expand_send_files(&format!("{}/sub/*.dat", root.display()), 100);
        assert_eq!(dat.len(), 1, "{dat:?}");

        assert!(wildcard_match("*.txt", "a.txt"));
        assert!(!wildcard_match("*.txt", "a.dat"));
        let _ = std::fs::remove_dir_all(&root);
    }
}

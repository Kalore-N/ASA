//! 外部音频工具封装（PCM 16-bit）。
//!
//! Linux 使用 `arecord` / `aplay` / `ffplay`；Windows 使用 `ffmpeg` / `ffplay`。
//! Android 不经过这里，使用 AudioRecord/AudioTrack。

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStderr, Command, Stdio};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};

use crate::config::VoicePrefs;
use crate::paths::RuntimeDirs;

pub fn frame_bytes(prefs: &VoicePrefs) -> usize {
    let rate = prefs.sample_rate.max(8000) as usize;
    let channels = prefs.channels.max(1) as usize;
    let ms = prefs.frame_ms.clamp(10, 60) as usize;
    (rate / 1000 * ms * channels * 2).max(2)
}

pub fn voice_dir(dirs: &RuntimeDirs, prefs: &VoicePrefs) -> PathBuf {
    prefs
        .record_dir
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(crate::util::expand_tilde)
        .unwrap_or_else(|| dirs.downloads.join("voice"))
}

/// 子进程 stderr 的增量采集，用于在录音/播放失败时给出真实原因。
#[derive(Clone, Default)]
pub struct ProcLog(Arc<Mutex<String>>);

impl ProcLog {
    pub fn tail(&self) -> String {
        let text = self.0.lock().map(|t| t.clone()).unwrap_or_default();
        let lines: Vec<&str> = text
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .collect();
        lines
            .iter()
            .find(|line| {
                let low = line.to_lowercase();
                low.contains("error")
                    || low.contains("cannot")
                    || low.contains("unable")
                    || low.contains("fail")
                    || line.contains("失败")
                    || line.contains("无法")
            })
            .or_else(|| lines.last())
            .map(|line| line.to_string())
            .unwrap_or_default()
    }
}

fn spawn_log(mut pipe: ChildStderr) -> ProcLog {
    let log = ProcLog::default();
    let out = log.clone();
    thread::spawn(move || {
        let mut chunk = [0u8; 256];
        loop {
            match pipe.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(read) => {
                    if let Ok(mut text) = out.0.lock() {
                        if text.len() < 2048 {
                            text.push_str(&String::from_utf8_lossy(&chunk[..read]));
                        }
                    }
                }
            }
        }
    });
    log
}

/// 等待子进程稳定启动；若它立刻退出（设备被占用、参数不支持等）就报错。
fn wait_started(child: &mut Child, log: &ProcLog, what: &str) -> Result<()> {
    for _ in 0..20 {
        match child.try_wait() {
            Ok(Some(status)) => {
                thread::sleep(Duration::from_millis(20));
                let detail = log.tail();
                if detail.is_empty() {
                    bail!("{what}启动后立即退出（{status}）");
                }
                bail!("{what}启动失败（{status}）: {detail}");
            }
            Ok(None) => thread::sleep(Duration::from_millis(10)),
            Err(err) => bail!("检查{what}状态失败: {err}"),
        }
    }
    Ok(())
}

pub struct AudioCapture {
    child: Child,
    rx: Receiver<Vec<u8>>,
    handle: Option<thread::JoinHandle<()>>,
    log: ProcLog,
}

impl AudioCapture {
    pub fn try_recv(&self) -> Option<Vec<u8>> {
        self.rx.try_recv().ok()
    }

    /// 录音进程意外退出时返回可读原因（用于提示“没有声音”而不是静默）。
    pub fn failure(&mut self) -> Option<String> {
        match self.child.try_wait() {
            Ok(Some(status)) => {
                let detail = self.log.tail();
                Some(if detail.is_empty() {
                    format!("录音进程已退出（{status}）")
                } else {
                    format!("录音进程已退出（{status}）: {detail}")
                })
            }
            _ => None,
        }
    }

    pub fn stop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for AudioCapture {
    fn drop(&mut self) {
        self.stop();
    }
}

pub struct AudioPlayer {
    child: Child,
    tx: Option<Sender<Vec<u8>>>,
    handle: Option<thread::JoinHandle<()>>,
    log: ProcLog,
}

impl AudioPlayer {
    pub fn play(&self, data: Vec<u8>) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(data);
        }
    }

    /// 播放进程意外退出时返回可读原因。
    pub fn failure(&mut self) -> Option<String> {
        match self.child.try_wait() {
            Ok(Some(status)) => {
                let detail = self.log.tail();
                Some(if detail.is_empty() {
                    format!("播放进程已退出（{status}）")
                } else {
                    format!("播放进程已退出（{status}）: {detail}")
                })
            }
            _ => None,
        }
    }

    pub fn stop(&mut self) {
        // 先断开播放线程的 channel，再杀子进程，避免 join 死锁。
        self.tx.take();
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for AudioPlayer {
    fn drop(&mut self) {
        self.stop();
    }
}

pub struct VoiceAudio {
    pub capture: AudioCapture,
    pub player: AudioPlayer,
    /// 实际使用的录音设备（便于诊断无声问题）。
    pub input_device: String,
    /// 实际使用的播放设备。
    pub output_device: String,
}

pub fn start(prefs: &VoicePrefs) -> Result<VoiceAudio> {
    if !prefs.enabled {
        bail!("语音功能未启用");
    }
    let frame = frame_bytes(prefs);
    let rate = prefs.sample_rate.max(8000).to_string();
    let channels = prefs.channels.max(1).to_string();

    let (mut capture, capture_log, input_device) = spawn_capture(prefs, &rate, &channels, None)?;
    let stdout = capture
        .stdout
        .take()
        .ok_or_else(|| anyhow!("录音工具没有 stdout"))?;
    let (tx, rx) = std::sync::mpsc::channel();
    let handle = thread::spawn(move || {
        let mut stdout = stdout;
        let mut buf = vec![0u8; frame];
        loop {
            match stdout.read_exact(&mut buf) {
                Ok(()) => {
                    if tx.send(buf.clone()).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    let (mut player, player_log, output_device) = match spawn_playback(prefs) {
        Ok(value) => value,
        Err(err) => {
            // Child 析构不会杀进程，这里必须自己清理，避免录音进程泄漏。
            let _ = capture.kill();
            let _ = capture.wait();
            return Err(err);
        }
    };
    let mut stdin = player
        .stdin
        .take()
        .ok_or_else(|| anyhow!("播放工具没有 stdin"))?;
    let (play_tx, play_rx) = std::sync::mpsc::channel::<Vec<u8>>();
    let play_handle = thread::spawn(move || {
        while let Ok(data) = play_rx.recv() {
            if stdin.write_all(&data).is_err() {
                break;
            }
            let _ = stdin.flush();
        }
    });

    Ok(VoiceAudio {
        capture: AudioCapture {
            child: capture,
            rx,
            handle: Some(handle),
            log: capture_log,
        },
        player: AudioPlayer {
            child: player,
            tx: Some(play_tx),
            handle: Some(play_handle),
            log: player_log,
        },
        input_device,
        output_device,
    })
}

/// 解析 `arecord -l` / `aplay -l` 输出中的 `plughw:card,device`。
#[cfg(target_os = "linux")]
fn detected_devices(tool: &str) -> Vec<String> {
    let mut child = match Command::new(tool)
        .arg("-l")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(_) => return Vec::new(),
    };
    let Some(mut stdout) = child.stdout.take() else {
        let _ = child.kill();
        return Vec::new();
    };
    let (tx, rx) = std::sync::mpsc::channel();
    thread::spawn(move || {
        let mut text = String::new();
        let _ = stdout.read_to_string(&mut text);
        let _ = tx.send(text);
    });
    // 某些声卡/驱动会让 `arecord -l` 长时间不退出，这里必须限时，不能阻塞 TUI。
    let text = match rx.recv_timeout(Duration::from_millis(1500)) {
        Ok(text) => {
            let _ = child.wait();
            text
        }
        Err(_) => {
            let _ = child.kill();
            let _ = child.wait();
            return Vec::new();
        }
    };
    let mut devices = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("card ") else {
            continue;
        };
        let Some((card, rest)) = rest.split_once(':') else {
            continue;
        };
        let Some((_, rest)) = rest.split_once(", device ") else {
            continue;
        };
        let Some((device, _)) = rest.split_once(':') else {
            continue;
        };
        let (Ok(card), Ok(device)) = (card.trim().parse::<u32>(), device.trim().parse::<u32>())
        else {
            continue;
        };
        let value = format!("plughw:{card},{device}");
        if !devices.contains(&value) {
            devices.push(value);
        }
    }
    devices
}

#[cfg(target_os = "linux")]
fn capture_devices(prefs: &VoicePrefs) -> Vec<String> {
    if let Some(device) = prefs
        .input_device
        .as_deref()
        .map(str::trim)
        .filter(|device| !device.is_empty())
    {
        return vec![device.to_string()];
    }
    // 先走声音服务（PipeWire/PulseAudio），再回退到空闲的硬件子设备。
    // `default` 在部分声卡上会直接命中被声音服务占用的 hw 设备，需要继续尝试。
    let mut devices = vec![
        "default".to_string(),
        "pulse".to_string(),
        "pipewire".to_string(),
    ];
    for device in detected_devices("arecord") {
        if !devices.contains(&device) {
            devices.push(device);
        }
    }
    devices
}

#[cfg(target_os = "linux")]
fn playback_devices(prefs: &VoicePrefs) -> Vec<String> {
    if let Some(device) = prefs
        .output_device
        .as_deref()
        .map(str::trim)
        .filter(|device| !device.is_empty())
    {
        return vec![device.to_string()];
    }
    let mut devices = vec![
        "default".to_string(),
        "pulse".to_string(),
        "pipewire".to_string(),
    ];
    for device in detected_devices("aplay") {
        if !devices.contains(&device) {
            devices.push(device);
        }
    }
    devices
}

#[cfg(target_os = "linux")]
fn spawn_capture(
    prefs: &VoicePrefs,
    rate: &str,
    channels: &str,
    wav: Option<&Path>,
) -> Result<(Child, ProcLog, String)> {
    let mut last: anyhow::Error = anyhow!("没有可用的录音设备");
    for device in capture_devices(prefs) {
        let mut cmd = Command::new("arecord");
        cmd.args(["-q", "-f", "S16_LE", "-r", rate, "-c", channels]);
        match wav {
            Some(path) => {
                cmd.args(["-t", "wav"]).arg(path);
            }
            None => {
                cmd.args(["-t", "raw"]);
            }
        }
        cmd.args(["-D", &device])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::null());
        let mut child = match cmd.spawn() {
            Ok(child) => child,
            Err(err) => {
                last = anyhow!("设备 {device}: 无法启动 arecord: {err}");
                continue;
            }
        };
        let log = child.stderr.take().map(spawn_log).unwrap_or_default();
        match wait_started(&mut child, &log, "录音工具") {
            Ok(()) => return Ok((child, log, device)),
            Err(err) => {
                let _ = child.kill();
                let _ = child.wait();
                last = anyhow!("设备 {device}: {err}");
            }
        }
    }
    Err(last.context("没有找到可用的录音设备，可在 voice.input_device 中指定"))
}

#[cfg(target_os = "linux")]
fn spawn_playback(prefs: &VoicePrefs) -> Result<(Child, ProcLog, String)> {
    let rate = prefs.sample_rate.max(8000).to_string();
    let channels = prefs.channels.max(1).to_string();
    let mut last: anyhow::Error = anyhow!("没有可用的播放设备");
    for device in playback_devices(prefs) {
        let mut cmd = Command::new("aplay");
        cmd.args([
            "-q", "-f", "S16_LE", "-r", &rate, "-c", &channels, "-t", "raw", "-D", &device,
        ]);
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        let mut child = match cmd.spawn() {
            Ok(child) => child,
            Err(err) => {
                last = anyhow!("设备 {device}: 无法启动 aplay: {err}");
                continue;
            }
        };
        let log = child.stderr.take().map(spawn_log).unwrap_or_default();
        match wait_started(&mut child, &log, "播放工具") {
            Ok(()) => return Ok((child, log, device)),
            Err(err) => {
                let _ = child.kill();
                let _ = child.wait();
                last = anyhow!("设备 {device}: {err}");
            }
        }
    }
    Err(last.context("没有找到可用的播放设备，可在 voice.output_device 中指定"))
}

#[cfg(target_os = "windows")]
fn spawn_capture(
    prefs: &VoicePrefs,
    rate: &str,
    channels: &str,
    wav: Option<&Path>,
) -> Result<(Child, ProcLog, String)> {
    let device = prefs
        .input_device
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("Windows 录音需要在 voice.input_device 填写 ffmpeg dshow 设备名"))?
        .to_string();
    let mut cmd = Command::new("ffmpeg");
    cmd.args([
        "-hide_banner",
        "-loglevel",
        "error",
        "-f",
        "dshow",
        "-i",
        &format!("audio={device}"),
        "-ac",
        channels,
        "-ar",
        rate,
    ]);
    match wav {
        Some(path) => {
            cmd.arg("-y").arg(path);
        }
        None => {
            cmd.args(["-f", "s16le", "-"]);
        }
    }
    cmd.stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null());
    let mut child = cmd.spawn().context("启动录音工具失败")?;
    let log = child.stderr.take().map(spawn_log).unwrap_or_default();
    wait_started(&mut child, &log, "录音工具")?;
    Ok((child, log, device))
}

#[cfg(target_os = "windows")]
fn spawn_playback(prefs: &VoicePrefs) -> Result<(Child, ProcLog, String)> {
    let rate = prefs.sample_rate.max(8000).to_string();
    let channels = prefs.channels.max(1).to_string();
    let mut cmd = Command::new("ffplay");
    cmd.args([
        "-hide_banner",
        "-loglevel",
        "quiet",
        "-f",
        "s16le",
        "-ar",
        &rate,
        "-ac",
        &channels,
        "-nodisp",
        "-autoexit",
        "-",
    ]);
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().context("启动播放工具失败")?;
    let log = child.stderr.take().map(spawn_log).unwrap_or_default();
    wait_started(&mut child, &log, "播放工具")?;
    Ok((child, log, String::from("ffplay default")))
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
fn spawn_capture(
    _prefs: &VoicePrefs,
    _rate: &str,
    _channels: &str,
    _wav: Option<&Path>,
) -> Result<(Child, ProcLog, String)> {
    bail!("当前平台不支持 TUI 语音采集")
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
fn spawn_playback(_prefs: &VoicePrefs) -> Result<(Child, ProcLog, String)> {
    bail!("当前平台不支持 TUI 语音播放")
}

/// 开始录制一个 WAV 语音消息。
pub fn start_recording(prefs: &VoicePrefs, path: &Path) -> Result<Child> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let rate = prefs.sample_rate.max(8000).to_string();
    let channels = prefs.channels.max(1).to_string();
    let (child, _log, _device) = spawn_capture(prefs, &rate, &channels, Some(path))?;
    Ok(child)
}

/// 播放一个 WAV/音频文件；不阻塞 TUI。
pub fn play_file(path: &Path) -> Result<Child> {
    #[cfg(target_os = "linux")]
    {
        Command::new("ffplay")
            .args(["-hide_banner", "-loglevel", "quiet", "-nodisp", "-autoexit"])
            .arg(path)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("启动 ffplay 失败")
    }
    #[cfg(target_os = "windows")]
    {
        Command::new("ffplay")
            .args(["-hide_banner", "-loglevel", "quiet", "-nodisp", "-autoexit"])
            .arg(path)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("启动 ffplay 失败")
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    {
        let _ = path;
        bail!("当前平台不支持音频播放")
    }
}

pub fn stop_recording(child: &mut Child) {
    #[cfg(unix)]
    {
        // SIGINT 让 arecord 正确写回 WAV 头。
        let _ = Command::new("kill")
            .args(["-INT", &child.id().to_string()])
            .status();
    }
    #[cfg(windows)]
    {
        let _ = Command::new("taskkill")
            .args(["/PID", &child.id().to_string()])
            .status();
    }
    let _ = child.wait();
}

pub fn check_tools() -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        if !command_exists("arecord") || !command_exists("aplay") {
            bail!("缺少 arecord/aplay，请安装 alsa-utils");
        }
        if !command_exists("ffplay") {
            bail!("缺少 ffplay，请安装 ffmpeg");
        }
    }
    #[cfg(target_os = "windows")]
    {
        if !command_exists("ffmpeg") || !command_exists("ffplay") {
            bail!("缺少 ffmpeg/ffplay，请安装并加入 PATH");
        }
    }
    Ok(())
}

#[allow(dead_code)]
fn command_exists(name: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| {
            std::env::split_paths(&paths).any(|dir| {
                let candidate = dir.join(name);
                candidate.is_file() || dir.join(format!("{name}.exe")).is_file()
            })
        })
        .unwrap_or(false)
}

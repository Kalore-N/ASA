//! 外部音频工具封装（PCM 16-bit）。
//!
//! Linux 使用 `arecord` / `aplay` / `ffplay`；Windows 使用 `ffmpeg` / `ffplay`。
//! Android 不经过这里，使用 AudioRecord/AudioTrack。

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{Receiver, Sender};
use std::thread;

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

pub struct AudioCapture {
    child: Child,
    rx: Receiver<Vec<u8>>,
    handle: Option<thread::JoinHandle<()>>,
}

impl AudioCapture {
    pub fn try_recv(&self) -> Option<Vec<u8>> {
        self.rx.try_recv().ok()
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
}

impl AudioPlayer {
    pub fn play(&self, data: Vec<u8>) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(data);
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
}

pub fn start(prefs: &VoicePrefs) -> Result<VoiceAudio> {
    if !prefs.enabled {
        bail!("语音功能未启用");
    }
    let frame = frame_bytes(prefs);
    let rate = prefs.sample_rate.max(8000).to_string();
    let channels = prefs.channels.max(1).to_string();

    let mut capture_cmd = capture_command(prefs, &rate, &channels)?;
    let mut capture = capture_cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .stdin(Stdio::null())
        .spawn()
        .context("启动录音工具失败")?;
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

    let mut player_cmd = playback_command(prefs)?;
    let mut player = player_cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("启动播放工具失败")?;
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
        },
        player: AudioPlayer {
            child: player,
            tx: Some(play_tx),
            handle: Some(play_handle),
        },
    })
}

fn capture_command(prefs: &VoicePrefs, rate: &str, channels: &str) -> Result<Command> {
    #[cfg(target_os = "linux")]
    {
        let mut cmd = Command::new("arecord");
        cmd.args([
            "-q", "-f", "S16_LE", "-r", rate, "-c", channels, "-t", "raw",
        ]);
        let device = prefs
            .input_device
            .as_deref()
            .map(str::trim)
            .filter(|d| !d.is_empty())
            .map(str::to_string)
            .or_else(default_capture_device);
        if let Some(device) = device {
            cmd.args(["-D", &device]);
        }
        Ok(cmd)
    }
    #[cfg(target_os = "windows")]
    {
        let device = prefs
            .input_device
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                anyhow!("Windows 录音需要在 voice.input_device 填写 ffmpeg dshow 设备名")
            })?;
        let mut cmd = Command::new("ffmpeg");
        cmd.args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "dshow",
            "-i",
            &format!("audio={device}"),
            "-f",
            "s16le",
            "-ac",
            channels,
            "-ar",
            rate,
            "-",
        ]);
        Ok(cmd)
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    {
        let _ = (prefs, rate, channels);
        bail!("当前平台不支持 TUI 语音采集")
    }
}

fn playback_command(prefs: &VoicePrefs) -> Result<Command> {
    let rate = prefs.sample_rate.max(8000).to_string();
    let channels = prefs.channels.max(1).to_string();
    #[cfg(target_os = "linux")]
    {
        let mut cmd = Command::new("aplay");
        cmd.args([
            "-q", "-f", "S16_LE", "-r", &rate, "-c", &channels, "-t", "raw",
        ]);
        if let Some(device) = prefs
            .output_device
            .as_deref()
            .filter(|d| !d.trim().is_empty())
        {
            cmd.args(["-D", device]);
        }
        Ok(cmd)
    }
    #[cfg(target_os = "windows")]
    {
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
        Ok(cmd)
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    {
        let _ = (prefs, rate, channels);
        bail!("当前平台不支持 TUI 语音播放")
    }
}

/// 开始录制一个 WAV 语音消息。
pub fn start_recording(prefs: &VoicePrefs, path: &Path) -> Result<Child> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let rate = prefs.sample_rate.max(8000).to_string();
    let channels = prefs.channels.max(1).to_string();
    #[cfg(target_os = "linux")]
    {
        let mut cmd = Command::new("arecord");
        cmd.args([
            "-q", "-f", "S16_LE", "-r", &rate, "-c", &channels, "-t", "wav",
        ])
        .arg(path);
        let device = prefs
            .input_device
            .as_deref()
            .map(str::trim)
            .filter(|d| !d.is_empty())
            .map(str::to_string)
            .or_else(default_capture_device);
        if let Some(device) = device {
            cmd.args(["-D", &device]);
        }
        return cmd.spawn().context("启动语音消息录音失败");
    }
    #[cfg(target_os = "windows")]
    {
        let device = prefs
            .input_device
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow!("Windows 录音需要配置 voice.input_device"))?;
        let mut cmd = Command::new("ffmpeg");
        cmd.args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "dshow",
            "-i",
            &format!("audio={device}"),
            "-ac",
            &channels,
            "-ar",
            &rate,
        ])
        .arg(path);
        return cmd.spawn().context("启动语音消息录音失败");
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    {
        let _ = (prefs, path, rate, channels);
        bail!("当前平台不支持语音消息录音")
    }
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

#[cfg(target_os = "linux")]
fn default_capture_device() -> Option<String> {
    let output = Command::new("arecord").arg("-l").output().ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    for line in text.lines() {
        let line = line.trim();
        if !line.starts_with("card ") {
            continue;
        }
        let card = line
            .strip_prefix("card ")?
            .split(':')
            .next()?
            .trim()
            .to_string();
        let device = line
            .split("device ")
            .nth(1)?
            .split(':')
            .next()?
            .trim()
            .to_string();
        return Some(format!("plughw:{card},{device}"));
    }
    None
}

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

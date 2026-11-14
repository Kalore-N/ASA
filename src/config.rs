//! 全局配置与按群配置。
//!
//! - `runtime/data/config.json`：全局默认值（下载目录、大小上限、UI、网络）。
//! - `runtime/data/groups/<room_id>.json`：群内昵称与可覆盖项。
//!
//! 群配置中的 `None` 表示继承全局配置。

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::paths::RuntimeDirs;
use crate::settings::Settings;
use crate::util;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_version")]
    pub version: u8,
    /// 下载目录；支持 `~`。为空时使用 `runtime/downloads`。
    #[serde(default)]
    pub download_dir: Option<String>,
    /// 单文件上限（MB），0 表示不限制。
    #[serde(default = "default_max_file_mb")]
    pub max_file_mb: u64,
    #[serde(default)]
    pub auto_accept: bool,
    #[serde(default)]
    pub ui: UiPrefs,
    #[serde(default)]
    pub network: NetworkPrefs,
    #[serde(default)]
    pub sync: SyncPrefs,
    #[serde(default)]
    pub easytier: EasyTierPrefs,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EasyTierPrefs {
    /// 用户导入的 EasyTier 公共节点地址。
    #[serde(default)]
    pub nodes: Vec<String>,
}

impl Default for EasyTierPrefs {
    fn default() -> Self {
        Self { nodes: Vec::new() }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UiPrefs {
    /// `dark` / `light` / `high-contrast`。
    #[serde(default = "default_theme")]
    pub theme: String,
    /// Android 字号；TUI 只保存偏好，终端字体由终端模拟器决定。
    #[serde(default = "default_font_size")]
    pub font_size: u16,
    #[serde(default)]
    pub compact: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SyncPrefs {
    /// 加入频道后自动向在线成员拉取最近消息。
    #[serde(default = "default_sync_enabled")]
    pub enabled: bool,
    /// 每次请求的最大消息条数。
    #[serde(default = "default_sync_count")]
    pub count: u32,
}

impl Default for SyncPrefs {
    fn default() -> Self {
        Self {
            enabled: default_sync_enabled(),
            count: default_sync_count(),
        }
    }
}

fn default_sync_enabled() -> bool {
    true
}

fn default_sync_count() -> u32 {
    50
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NetworkPrefs {
    /// 监听端口；为空时沿用旧 settings.json 或 7777。
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default = "default_mdns")]
    pub mdns: bool,
    /// 中继地址列表；默认留空，局域网靠 mDNS，跨网部署中继后再添加。
    #[serde(default)]
    pub relays: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: default_version(),
            download_dir: None,
            max_file_mb: default_max_file_mb(),
            auto_accept: false,
            ui: UiPrefs::default(),
            network: NetworkPrefs::default(),
            sync: SyncPrefs::default(),
            easytier: EasyTierPrefs::default(),
        }
    }
}

impl Default for UiPrefs {
    fn default() -> Self {
        Self {
            theme: default_theme(),
            font_size: default_font_size(),
            compact: false,
        }
    }
}

impl Default for NetworkPrefs {
    fn default() -> Self {
        Self {
            port: None,
            mdns: default_mdns(),
            relays: Vec::new(),
        }
    }
}

fn default_version() -> u8 {
    1
}
fn default_max_file_mb() -> u64 {
    100
}
fn default_theme() -> String {
    "dark".to_string()
}
fn default_font_size() -> u16 {
    15
}
fn default_mdns() -> bool {
    true
}

pub fn config_path(dirs: &RuntimeDirs) -> PathBuf {
    dirs.data.join("config.json")
}

pub fn groups_dir(dirs: &RuntimeDirs) -> PathBuf {
    dirs.data.join("groups")
}

pub fn group_path(dirs: &RuntimeDirs, room_id: &str) -> PathBuf {
    groups_dir(dirs).join(format!("{room_id}.json"))
}

impl Config {
    pub fn load(path: &Path) -> Result<Option<Self>> {
        if !path.exists() {
            return Ok(None);
        }
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("读取配置失败: {}", path.display()))?;
        let config: Self = serde_json::from_str(&text)
            .with_context(|| format!("配置格式错误: {}", path.display()))?;
        Ok(Some(config))
    }

    /// 读取配置；不存在时生成默认配置并写盘（会迁移旧 settings.json 的端口）。
    pub fn ensure(dirs: &RuntimeDirs) -> Result<Self> {
        let path = config_path(dirs);
        if let Some(config) = Self::load(&path)? {
            return Ok(config);
        }
        let mut config = Config::default();
        if let Ok(settings) = Settings::load(&dirs.settings_path()) {
            config.network.port = settings.port;
        }
        config.save(&path)?;
        Ok(config)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("创建配置目录失败: {}", parent.display()))?;
        }
        std::fs::write(path, serde_json::to_string_pretty(self)?)
            .with_context(|| format!("写入配置失败: {}", path.display()))?;
        restrict_permissions(path);
        Ok(())
    }

    /// 解析下载目录；不存在时回退到 `runtime/downloads`。
    pub fn download_dir(&self, dirs: &RuntimeDirs) -> PathBuf {
        match self.download_dir.as_deref().map(str::trim) {
            Some(value) if !value.is_empty() => util::expand_tilde(value),
            _ => dirs.downloads.clone(),
        }
    }

    pub fn max_file(&self) -> u64 {
        match self.max_file_mb {
            0 => u64::MAX,
            mb => mb.saturating_mul(1024 * 1024),
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct GroupProfile {
    /// 本机显示的群名称，只保存在本地，不影响其他成员。
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub nick: Option<String>,
    #[serde(default)]
    pub download_dir: Option<String>,
    #[serde(default)]
    pub max_file_mb: Option<u64>,
    #[serde(default)]
    pub auto_accept: Option<bool>,
    #[serde(default)]
    pub notify_mentions: Option<bool>,
    /// 是否为此频道开启 EasyTier 内网穿透。
    #[serde(default)]
    pub easytier_enabled: Option<bool>,
}

impl GroupProfile {
    pub fn load(dirs: &RuntimeDirs, room_id: &str) -> Result<Self> {
        Self::load_at(&group_path(dirs, room_id))
    }

    pub fn load_at(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("读取群配置失败: {}", path.display()))?;
        let profile: Self = serde_json::from_str(&text)
            .with_context(|| format!("群配置格式错误: {}", path.display()))?;
        Ok(profile)
    }

    pub fn save(&self, dirs: &RuntimeDirs, room_id: &str) -> Result<()> {
        self.save_at(&group_path(dirs, room_id))
    }

    pub fn save_at(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("创建群配置目录失败: {}", parent.display()))?;
        }
        std::fs::write(path, serde_json::to_string_pretty(self)?)
            .with_context(|| format!("写入群配置失败: {}", path.display()))?;
        restrict_permissions(path);
        Ok(())
    }

    pub fn download_dir(&self, config: &Config, dirs: &RuntimeDirs) -> PathBuf {
        match self.download_dir.as_deref().map(str::trim) {
            Some(value) if !value.is_empty() => util::expand_tilde(value),
            _ => config.download_dir(dirs),
        }
    }

    pub fn max_file(&self, config: &Config) -> u64 {
        match self.max_file_mb {
            Some(0) => u64::MAX,
            Some(mb) => mb.saturating_mul(1024 * 1024),
            None => config.max_file(),
        }
    }

    pub fn auto_accept(&self, config: &Config) -> bool {
        self.auto_accept.unwrap_or(config.auto_accept)
    }

    pub fn notify_mentions(&self) -> bool {
        self.notify_mentions.unwrap_or(true)
    }
}

#[cfg(unix)]
fn restrict_permissions(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
fn restrict_permissions(_path: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn dirs(tag: &str) -> RuntimeDirs {
        let root = std::env::temp_dir().join(format!("asa-config-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        RuntimeDirs::ensure(&root).unwrap()
    }

    #[test]
    fn default_config_is_usable() {
        let config = Config::default();
        assert_eq!(config.max_file_mb, 100);
        assert!(config.network.mdns);
        assert!(config.network.relays.is_empty());
        assert_eq!(config.ui.theme, "dark");
        assert_eq!(config.ui.font_size, 15);
        assert!(config.sync.enabled);
        assert_eq!(config.sync.count, 50);
    }

    #[test]
    fn config_roundtrip() {
        let dirs = dirs("roundtrip");
        let mut config = Config::default();
        config.download_dir = Some("~/Downloads/asa".to_string());
        config.max_file_mb = 250;
        config.ui.theme = "light".to_string();
        config.network.relays = vec!["relay.example:9000".to_string()];
        let path = config_path(&dirs);
        config.save(&path).unwrap();
        let loaded = Config::load(&path).unwrap().unwrap();
        assert_eq!(loaded.download_dir.as_deref(), Some("~/Downloads/asa"));
        assert_eq!(loaded.max_file_mb, 250);
        assert_eq!(loaded.ui.theme, "light");
        assert_eq!(loaded.network.relays, vec!["relay.example:9000"]);
    }

    #[test]
    fn group_profile_overrides_config() {
        let dirs = dirs("group");
        let mut config = Config::default();
        config.max_file_mb = 100;
        let mut profile = GroupProfile::default();
        profile.nick = Some("alice".to_string());
        profile.max_file_mb = Some(10);
        assert_eq!(profile.max_file(&config), 10 * 1024 * 1024);
        assert_eq!(profile.auto_accept(&config), false);
        assert!(profile.notify_mentions());
        profile.save(&dirs, "room1").unwrap();
        let loaded = GroupProfile::load(&dirs, "room1").unwrap();
        assert_eq!(loaded.nick.as_deref(), Some("alice"));
        assert_eq!(loaded.max_file_mb, Some(10));
    }

    #[test]
    fn ensure_migrates_old_settings_port() {
        let dirs = dirs("migrate");
        Settings { port: Some(8123) }
            .save(&dirs.settings_path())
            .unwrap();
        let config = Config::ensure(&dirs).unwrap();
        assert_eq!(config.network.port, Some(8123));
        assert!(config_path(&dirs).exists());
    }

    #[test]
    fn group_name_is_local_and_persisted() {
        let dirs = dirs("group-name");
        let mut profile = GroupProfile::default();
        profile.name = Some("我的群".to_string());
        profile.save(&dirs, "room-name").unwrap();
        let loaded = GroupProfile::load(&dirs, "room-name").unwrap();
        assert_eq!(loaded.name.as_deref(), Some("我的群"));
    }
}

//! 数据目录定位。
//!
//! - 默认使用标准位置：配置 `~/.config/asa`（遵循 `XDG_CONFIG_HOME`），
//!   日志 `~/.local/state/asa/logs`，下载 `~/Downloads`。
//! - 显式 `--data-dir <路径>` 或 Android 端仍使用 `<路径>/runtime` 的自包含布局。

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// 用户主目录。
pub fn home_dir() -> Result<PathBuf> {
    if let Some(home) = std::env::var_os("HOME") {
        if !home.is_empty() {
            return Ok(PathBuf::from(home));
        }
    }
    if let Some(home) = std::env::var_os("USERPROFILE") {
        if !home.is_empty() {
            return Ok(PathBuf::from(home));
        }
    }
    #[cfg(windows)]
    {
        let drive = std::env::var_os("HOMEDRIVE");
        let path = std::env::var_os("HOMEPATH");
        if let (Some(d), Some(p)) = (drive, path) {
            return Ok(PathBuf::from(d).join(p));
        }
    }
    anyhow::bail!("无法定位用户主目录（HOME / USERPROFILE 均未设置）")
}

/// 默认数据根目录 `~/Documents/ASA`。
///
/// Windows 上若 Documents 被 OneDrive 重定向，会优先使用实际存在的目录。
pub fn default_root() -> Result<PathBuf> {
    let home = home_dir()?;
    let documents = home.join("Documents");
    if documents.is_dir() {
        return Ok(documents.join("ASA"));
    }
    let onedrive = home.join("OneDrive").join("Documents");
    if onedrive.is_dir() {
        return Ok(onedrive.join("ASA"));
    }
    Ok(documents.join("ASA"))
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn xdg_dir(var: &str, fallback: impl FnOnce() -> Result<PathBuf>) -> Result<PathBuf> {
    if let Some(value) = std::env::var_os(var) {
        if !value.is_empty() {
            let path = PathBuf::from(value);
            if path.is_absolute() {
                return Ok(path.join("asa"));
            }
        }
    }
    fallback()
}

/// 标准配置目录：Linux `~/.config/asa`；Windows `%APPDATA%\asa`；
/// macOS `~/Library/Application Support/asa`。
pub fn config_dir() -> Result<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        if let Some(base) = std::env::var_os("APPDATA") {
            if !base.is_empty() {
                return Ok(PathBuf::from(base).join("asa"));
            }
        }
        return Ok(home_dir()?.join("AppData").join("Roaming").join("asa"));
    }
    #[cfg(target_os = "macos")]
    {
        return Ok(home_dir()?
            .join("Library")
            .join("Application Support")
            .join("asa"));
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        xdg_dir("XDG_CONFIG_HOME", || {
            Ok(home_dir()?.join(".config").join("asa"))
        })
    }
}

/// 标准状态目录（日志）：Linux `~/.local/state/asa`；
/// Windows `%LOCALAPPDATA%\asa`；macOS `~/Library/Logs/asa`。
pub fn state_dir() -> Result<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        if let Some(base) = std::env::var_os("LOCALAPPDATA") {
            if !base.is_empty() {
                return Ok(PathBuf::from(base).join("asa"));
            }
        }
        return Ok(home_dir()?.join("AppData").join("Local").join("asa"));
    }
    #[cfg(target_os = "macos")]
    {
        return Ok(home_dir()?.join("Library").join("Logs").join("asa"));
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        xdg_dir("XDG_STATE_HOME", || {
            Ok(home_dir()?.join(".local").join("state").join("asa"))
        })
    }
}

/// 标准缓存目录：Linux `~/.cache/asa`；Windows `%LOCALAPPDATA%\asa\cache`；
/// macOS `~/Library/Caches/asa`。
pub fn cache_dir() -> Result<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        return Ok(state_dir()?.join("cache"));
    }
    #[cfg(target_os = "macos")]
    {
        return Ok(home_dir()?.join("Library").join("Caches").join("asa"));
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        xdg_dir("XDG_CACHE_HOME", || {
            Ok(home_dir()?.join(".cache").join("asa"))
        })
    }
}

/// 下载目录：Linux 读取 `user-dirs.dirs` 的 `XDG_DOWNLOAD_DIR`，
/// 其他平台使用 `~/Downloads`。
pub fn downloads_dir() -> Result<PathBuf> {
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    {
        return Ok(home_dir()?.join("Downloads"));
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let config_home = match std::env::var_os("XDG_CONFIG_HOME") {
            Some(value) if !value.is_empty() => PathBuf::from(value),
            _ => home_dir()?.join(".config"),
        };
        let file = config_home.join("user-dirs.dirs");
        if let Ok(text) = std::fs::read_to_string(&file) {
            for line in text.lines() {
                let line = line.trim();
                let Some(value) = line.strip_prefix("XDG_DOWNLOAD_DIR=") else {
                    continue;
                };
                let value = value.trim().trim_matches('"');
                if value.is_empty() {
                    continue;
                }
                let home = home_dir()?;
                let expanded = value
                    .replace("$HOME", &home.display().to_string())
                    .replace("${HOME}", &home.display().to_string());
                let path = PathBuf::from(expanded);
                if path.is_absolute() {
                    return Ok(path);
                }
            }
        }
        Ok(home_dir()?.join("Downloads"))
    }
}

/// 运行时目录集合。
#[derive(Clone, Debug)]
pub struct RuntimeDirs {
    pub root: PathBuf,
    pub data: PathBuf,
    pub logs: PathBuf,
    pub downloads: PathBuf,
    pub tmp: PathBuf,
}

impl RuntimeDirs {
    /// 在 `<root>/runtime` 下创建并返回各子目录（显式数据目录 / Android）。
    pub fn ensure(root: &Path) -> Result<Self> {
        let runtime = root.join("runtime");
        let dirs = RuntimeDirs {
            root: runtime.clone(),
            data: runtime.join("data"),
            logs: runtime.join("logs"),
            downloads: runtime.join("downloads"),
            tmp: runtime.join("tmp"),
        };
        for dir in [&dirs.data, &dirs.logs, &dirs.downloads, &dirs.tmp] {
            std::fs::create_dir_all(dir)
                .with_context(|| format!("创建目录失败: {}", dir.display()))?;
        }
        Ok(dirs)
    }

    /// 标准 Linux/桌面布局：
    /// 配置在 `~/.config/asa`，日志在 `~/.local/state/asa/logs`，
    /// 下载在 `~/Downloads`，临时文件在 `~/.cache/asa/tmp`。
    pub fn standard() -> Result<Self> {
        let data = config_dir()?;
        let dirs = RuntimeDirs {
            root: data.clone(),
            data,
            logs: state_dir()?.join("logs"),
            downloads: downloads_dir()?,
            tmp: cache_dir()?.join("tmp"),
        };
        for dir in [&dirs.data, &dirs.logs, &dirs.downloads, &dirs.tmp] {
            std::fs::create_dir_all(dir)
                .with_context(|| format!("创建目录失败: {}", dir.display()))?;
        }
        migrate_legacy(&dirs);
        Ok(dirs)
    }

    pub fn identity_path(&self) -> PathBuf {
        self.data.join("identity.json")
    }

    pub fn room_path(&self) -> PathBuf {
        self.data.join("room.json")
    }

    pub fn settings_path(&self) -> PathBuf {
        self.data.join("settings.json")
    }
}

fn copy_dir_merge(src: &Path, dst: &Path) {
    if !src.is_dir() {
        return;
    }
    let Ok(entries) = std::fs::read_dir(src) else {
        return;
    };
    for entry in entries.flatten() {
        let target = dst.join(entry.file_name());
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            let _ = std::fs::create_dir_all(&target);
            copy_dir_merge(&entry.path(), &target);
        } else if kind.is_file() && !target.exists() {
            if std::fs::copy(entry.path(), &target).is_ok() {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let _ =
                        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600));
                }
            }
        }
    }
}

/// 首次启用标准目录时，把旧版 `~/Documents/ASA/runtime` 的数据复制过来。
fn migrate_legacy(dirs: &RuntimeDirs) {
    if dirs.data.join("config.json").exists() || dirs.data.join("identity.json").exists() {
        return;
    }
    let Ok(legacy_root) = default_root() else {
        return;
    };
    let legacy = legacy_root.join("runtime");
    let legacy_data = legacy.join("data");
    if !legacy_data.join("identity.json").exists() && !legacy_data.join("config.json").exists() {
        return;
    }
    copy_dir_merge(&legacy_data, &dirs.data);
    copy_dir_merge(&legacy.join("logs"), &dirs.logs);
    eprintln!(
        "已将旧配置从 {} 迁移到 {}（下载文件仍保留在原位置）",
        legacy_data.display(),
        dirs.data.display()
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_dirs_are_created() {
        let dir = std::env::temp_dir().join(format!("asa-paths-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let dirs = RuntimeDirs::ensure(&dir).unwrap();
        assert!(dirs.data.is_dir());
        assert!(dirs.logs.is_dir());
        assert!(dirs.downloads.is_dir());
        assert!(dirs.tmp.is_dir());
        assert_eq!(dirs.identity_path(), dirs.data.join("identity.json"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

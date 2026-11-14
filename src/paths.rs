//! 数据目录定位。程序所有运行时数据都写在 `~/Documents/ASA/runtime` 下。

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
    /// 在 `<root>/runtime` 下创建并返回各子目录。
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

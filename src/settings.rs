//! 本机设置（目前只有监听端口），存放在 `runtime/data/settings.json`。

use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub port: Option<u16>,
}

impl Settings {
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("读取设置失败: {}", path.display()))?;
        let settings = serde_json::from_str(&text)
            .with_context(|| format!("设置文件损坏: {}", path.display()))?;
        Ok(settings)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, serde_json::to_string_pretty(self)?)
            .with_context(|| format!("写入设置失败: {}", path.display()))?;
        Ok(())
    }
}

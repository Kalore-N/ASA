//! 多群存储：每台设备可保存多个群，并记住最近使用的群。
//!
//! - `runtime/data/rooms/<room_id>.json`：每个群的配置（含密钥）。
//! - `runtime/data/groups/index.json`：群列表索引、当前群、最近使用时间。
//! - `runtime/data/room.json`：旧版单群文件，首次启动自动迁移。

use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::config::{self, GroupProfile};
use crate::paths::RuntimeDirs;
use crate::room::RoomConfig;
use crate::util::now_ms;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SavedGroup {
    pub id: String,
    pub name: String,
    pub last_used: u64,
    pub active: bool,
}

#[derive(Serialize, Deserialize, Default)]
struct StoredIndex {
    #[serde(default = "default_version")]
    version: u8,
    #[serde(default)]
    active: Option<String>,
    #[serde(default)]
    groups: Vec<StoredGroup>,
}

#[derive(Serialize, Deserialize, Clone, Default)]
struct StoredGroup {
    id: String,
    #[serde(default)]
    last_used: u64,
}

fn default_version() -> u8 {
    1
}

pub fn rooms_dir(dirs: &RuntimeDirs) -> PathBuf {
    dirs.data.join("rooms")
}

pub fn room_path(dirs: &RuntimeDirs, room_id: &str) -> PathBuf {
    rooms_dir(dirs).join(format!("{room_id}.json"))
}

fn index_path(dirs: &RuntimeDirs) -> PathBuf {
    dirs.data.join("groups").join("index.json")
}

fn legacy_path(dirs: &RuntimeDirs) -> PathBuf {
    dirs.room_path()
}

fn read_index(dirs: &RuntimeDirs) -> StoredIndex {
    let path = index_path(dirs);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return StoredIndex::default();
    };
    serde_json::from_str(&text).unwrap_or_default()
}

fn write_index(dirs: &RuntimeDirs, index: &StoredIndex) -> Result<()> {
    let path = index_path(dirs);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("创建群索引目录失败: {}", parent.display()))?;
    }
    std::fs::write(&path, serde_json::to_string_pretty(index)?)
        .with_context(|| format!("写入群索引失败: {}", path.display()))?;
    Ok(())
}

/// 把旧版单群 room.json 迁移到 rooms/ 目录。
pub fn migrate(dirs: &RuntimeDirs) -> Result<()> {
    let dir = rooms_dir(dirs);
    std::fs::create_dir_all(&dir).with_context(|| format!("创建群目录失败: {}", dir.display()))?;
    let has_rooms = std::fs::read_dir(&dir)
        .map(|entries| {
            entries
                .flatten()
                .any(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
        })
        .unwrap_or(false);
    if has_rooms {
        return Ok(());
    }
    let Some(room) = RoomConfig::load(&legacy_path(dirs))? else {
        return Ok(());
    };
    save(dirs, &room)?;
    Ok(())
}

pub fn load(dirs: &RuntimeDirs, room_id: &str) -> Result<Option<RoomConfig>> {
    let path = room_path(dirs, room_id);
    if path.exists() {
        return RoomConfig::load(&path);
    }
    // 兼容旧版单群文件。
    if let Some(room) = RoomConfig::load(&legacy_path(dirs))? {
        if room.id == room_id {
            return Ok(Some(room));
        }
    }
    Ok(None)
}

pub fn save(dirs: &RuntimeDirs, room: &RoomConfig) -> Result<()> {
    let dir = rooms_dir(dirs);
    std::fs::create_dir_all(&dir).with_context(|| format!("创建群目录失败: {}", dir.display()))?;
    room.save(&room_path(dirs, &room.id))?;
    touch(dirs, &room.id)?;
    Ok(())
}

pub fn touch(dirs: &RuntimeDirs, room_id: &str) -> Result<()> {
    let mut index = read_index(dirs);
    index.active = Some(room_id.to_string());
    if let Some(entry) = index.groups.iter_mut().find(|entry| entry.id == room_id) {
        entry.last_used = now_ms();
    } else {
        index.groups.push(StoredGroup {
            id: room_id.to_string(),
            last_used: now_ms(),
        });
    }
    write_index(dirs, &index)
}

pub fn set_active(dirs: &RuntimeDirs, room_id: &str) -> Result<()> {
    touch(dirs, room_id)
}

/// 列出所有保存的群，按最近使用时间倒序。
pub fn list(dirs: &RuntimeDirs) -> Result<Vec<SavedGroup>> {
    migrate(dirs)?;
    let index = read_index(dirs);
    let last_used: std::collections::HashMap<&str, u64> = index
        .groups
        .iter()
        .map(|entry| (entry.id.as_str(), entry.last_used))
        .collect();
    let mut groups = Vec::new();
    let dir = rooms_dir(dirs);
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            let Ok(Some(room)) = RoomConfig::load(&path) else {
                continue;
            };
            let profile =
                GroupProfile::load_at(&config::group_path(dirs, &room.id)).unwrap_or_default();
            let name = profile
                .name
                .filter(|name| !name.trim().is_empty())
                .unwrap_or_else(|| format!("群 {}", room.short_id()));
            groups.push(SavedGroup {
                id: room.id.clone(),
                name,
                last_used: last_used.get(room.id.as_str()).copied().unwrap_or(0),
                active: index.active.as_deref() == Some(room.id.as_str()),
            });
        }
    }
    groups.sort_by(|a, b| {
        b.active
            .cmp(&a.active)
            .then_with(|| b.last_used.cmp(&a.last_used))
            .then_with(|| a.name.cmp(&b.name))
    });
    Ok(groups)
}

/// 当前群：索引里的 active，否则最近使用的一个，否则旧版单群。
pub fn active(dirs: &RuntimeDirs) -> Result<Option<RoomConfig>> {
    migrate(dirs)?;
    let index = read_index(dirs);
    if let Some(id) = index.active.as_deref() {
        if let Some(room) = load(dirs, id)? {
            return Ok(Some(room));
        }
    }
    if let Some(group) = list(dirs)?.into_iter().next() {
        return load(dirs, &group.id);
    }
    RoomConfig::load(&legacy_path(dirs))
}

/// 删除本地保存的群（不删除聊天日志和下载文件）。
pub fn remove(dirs: &RuntimeDirs, room_id: &str) -> Result<()> {
    let path = room_path(dirs, room_id);
    if path.exists() {
        std::fs::remove_file(&path)
            .with_context(|| format!("删除群配置失败: {}", path.display()))?;
    }
    let mut index = read_index(dirs);
    index.groups.retain(|entry| entry.id != room_id);
    if index.active.as_deref() == Some(room_id) {
        index.active = index.groups.first().map(|entry| entry.id.clone());
    }
    write_index(dirs, &index)
}

pub fn path_for(dirs: &RuntimeDirs, room_id: &str) -> PathBuf {
    room_path(dirs, room_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::Identity;

    fn dirs(tag: &str) -> RuntimeDirs {
        let root = std::env::temp_dir().join(format!("asa-rooms-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        RuntimeDirs::ensure(&root).unwrap()
    }

    #[test]
    fn migrates_legacy_room() {
        let dirs = dirs("migrate");
        let identity = Identity::generate();
        let room = RoomConfig::create(&identity);
        room.save(&legacy_path(&dirs)).unwrap();

        migrate(&dirs).unwrap();
        assert!(room_path(&dirs, &room.id).exists());
        assert_eq!(active(&dirs).unwrap().unwrap().id, room.id);
    }

    #[test]
    fn saves_multiple_and_switches_active() {
        let dirs = dirs("multi");
        let identity = Identity::generate();
        let first = RoomConfig::create(&identity);
        let second = RoomConfig::create(&identity);
        save(&dirs, &first).unwrap();
        save(&dirs, &second).unwrap();

        let groups = list(&dirs).unwrap();
        assert_eq!(groups.len(), 2);
        assert_eq!(active(&dirs).unwrap().unwrap().id, second.id);
        assert!(
            groups
                .iter()
                .find(|group| group.id == second.id)
                .unwrap()
                .active
        );

        touch(&dirs, &first.id).unwrap();
        assert_eq!(active(&dirs).unwrap().unwrap().id, first.id);
    }

    #[test]
    fn list_uses_local_group_name() {
        let dirs = dirs("name");
        let identity = Identity::generate();
        let room = RoomConfig::create(&identity);
        save(&dirs, &room).unwrap();
        let mut profile = GroupProfile::default();
        profile.name = Some("研发群".to_string());
        profile.save(&dirs, &room.id).unwrap();

        let groups = list(&dirs).unwrap();
        assert_eq!(groups[0].name, "研发群");
    }

    #[test]
    fn remove_group_updates_active() {
        let dirs = dirs("remove");
        let identity = Identity::generate();
        let first = RoomConfig::create(&identity);
        let second = RoomConfig::create(&identity);
        save(&dirs, &first).unwrap();
        save(&dirs, &second).unwrap();
        remove(&dirs, &second.id).unwrap();
        let groups = list(&dirs).unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].id, first.id);
    }

    #[test]
    fn legacy_path_is_still_readable() {
        let dirs = dirs("legacy");
        let identity = Identity::generate();
        let room = RoomConfig::create(&identity);
        room.save(&legacy_path(&dirs)).unwrap();
        assert!(load(&dirs, &room.id).unwrap().is_some());
    }
}

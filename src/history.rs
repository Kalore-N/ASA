//! 本地聊天记录：按日期追加写 JSON Lines。
//!
//! 遵循「仅本地记录」策略：不向新成员补发历史。

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::proto::ReplyRef;
use crate::util::now_ms;

#[derive(Serialize)]
struct Record<'a> {
    ts: u64,
    kind: &'a str,
    /// 昵称（展示用）。
    from: &'a str,
    /// 短 ID（兼容旧日志与展示）。
    from_id: &'a str,
    /// 完整 64 位公钥；旧日志没有该字段。
    #[serde(skip_serializing_if = "Option::is_none")]
    pubkey: Option<&'a str>,
    text: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reply: Option<&'a ReplyRef>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    mentions: Vec<String>,
}

#[derive(Deserialize)]
struct StoredRecord {
    #[serde(default)]
    ts: u64,
    #[serde(default)]
    kind: String,
    #[serde(default)]
    from: String,
    #[serde(default)]
    from_id: String,
    #[serde(default)]
    pubkey: Option<String>,
    #[serde(default)]
    text: String,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    reply: Option<ReplyRef>,
    #[serde(default)]
    mentions: Vec<String>,
}

/// 从本地 JSONL 读出的聊天记录。
#[derive(Clone, Debug)]
pub struct StoredChat {
    pub ts: u64,
    pub from: String,
    pub from_id: String,
    pub pubkey: Option<String>,
    pub text: String,
    pub id: String,
    pub reply: Option<ReplyRef>,
    pub mentions: Vec<String>,
}

pub struct History {
    dir: PathBuf,
    room: String,
}

impl History {
    pub fn new(dir: PathBuf, room: String) -> Result<Self> {
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("创建日志目录失败: {}", dir.display()))?;
        Ok(Self { dir, room })
    }

    /// 今天的日志文件路径。
    pub fn current_path(&self) -> PathBuf {
        let date = chrono::Local::now().format("%Y-%m-%d");
        self.dir.join(format!("asa-{}-{}.jsonl", self.room, date))
    }

    /// 追加一条记录；失败时静默忽略，聊天不因日志问题中断。
    pub fn append(&self, kind: &str, from: &str, from_id: &str, text: &str) {
        self.write(Record {
            ts: now_ms(),
            kind,
            from,
            from_id,
            pubkey: None,
            text,
            id: None,
            reply: None,
            mentions: Vec::new(),
        });
    }

    /// 追加一条聊天记录，携带消息 id、回复引用和提及列表。
    #[allow(clippy::too_many_arguments)]
    pub fn append_chat(
        &self,
        from: &str,
        from_id: &str,
        pubkey: &str,
        text: &str,
        id: &str,
        reply: Option<&ReplyRef>,
        mentions: &[String],
    ) {
        self.append_chat_at(from, from_id, pubkey, text, id, now_ms(), reply, mentions);
    }

    /// 追加一条带原始时间戳的聊天记录（用于历史同步）。
    #[allow(clippy::too_many_arguments)]
    pub fn append_chat_at(
        &self,
        from: &str,
        from_id: &str,
        pubkey: &str,
        text: &str,
        id: &str,
        ts: u64,
        reply: Option<&ReplyRef>,
        mentions: &[String],
    ) {
        self.write(Record {
            ts,
            kind: "chat",
            from,
            from_id,
            pubkey: Some(pubkey),
            text,
            id: Some(id),
            reply,
            mentions: mentions.to_vec(),
        });
    }

    /// 读取当前频道的全部聊天记录（按时间排序、按消息 id 去重）。
    pub fn load_chats(&self) -> Vec<StoredChat> {
        let mut by_id: HashMap<String, StoredChat> = HashMap::new();
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        let prefix = format!("asa-{}-", self.room);
        let mut paths: Vec<PathBuf> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension().and_then(|ext| ext.to_str()) == Some("jsonl")
                    && path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .map(|name| name.starts_with(&prefix))
                        .unwrap_or(false)
            })
            .collect();
        paths.sort();
        for path in paths {
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            for line in text.lines() {
                let Ok(record) = serde_json::from_str::<StoredRecord>(line) else {
                    continue;
                };
                if record.kind != "chat" || record.text.is_empty() {
                    continue;
                }
                let Some(id) = record.id.filter(|id| !id.is_empty()) else {
                    continue;
                };
                by_id.insert(
                    id.clone(),
                    StoredChat {
                        ts: record.ts,
                        from: record.from,
                        from_id: record.from_id,
                        pubkey: record.pubkey.filter(|key| key.len() == 64),
                        text: record.text,
                        id,
                        reply: record.reply,
                        mentions: record.mentions,
                    },
                );
            }
        }
        let mut chats: Vec<StoredChat> = by_id.into_values().collect();
        chats.sort_by(|a, b| a.ts.cmp(&b.ts).then_with(|| a.id.cmp(&b.id)));
        chats
    }

    /// 读取最近的聊天记录；`lookup` 用于把旧日志里的短 ID 映射到完整公钥。
    pub fn load_recent(
        &self,
        limit: usize,
        lookup: impl Fn(&str) -> Option<String>,
    ) -> Vec<StoredChat> {
        let mut chats = self.load_chats();
        for chat in &mut chats {
            if chat.pubkey.is_none() {
                chat.pubkey = lookup(&chat.from_id);
            }
        }
        if limit > 0 && chats.len() > limit {
            chats.drain(..chats.len() - limit);
        }
        chats
    }

    fn write(&self, record: Record<'_>) {
        let Ok(line) = serde_json::to_string(&record) else {
            return;
        };
        let path = self.current_path();
        let _ = append_line(&path, &line);
    }
}

fn append_line(path: &Path, line: &str) -> Result<()> {
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("打开日志失败: {}", path.display()))?;
    writeln!(file, "{line}")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appends_jsonl_lines() {
        let dir = std::env::temp_dir().join(format!("asa-history-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let history = History::new(dir.clone(), "abc".to_string()).unwrap();
        history.append("chat", "alice", "aaaa", "hi");
        history.append("system", "system", "", "bob 上线了");
        let text = std::fs::read_to_string(history.current_path()).unwrap();
        assert_eq!(text.lines().count(), 2);
        assert!(text.contains("\"text\":\"hi\""));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn chat_record_keeps_reply_and_mentions() {
        let dir = std::env::temp_dir().join(format!("asa-history-chat-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let history = History::new(dir.clone(), "abc".to_string()).unwrap();
        let reply = ReplyRef {
            id: "deadbeef".to_string(),
            nick: "alice".to_string(),
            preview: "你好".to_string(),
        };
        let pubkey = "b".repeat(64);
        history.append_chat(
            "bob",
            "b0b",
            &pubkey,
            "收到",
            "cafebabe",
            Some(&reply),
            &["alice-pub".to_string()],
        );
        let text = std::fs::read_to_string(history.current_path()).unwrap();
        assert!(text.contains("\"id\":\"cafebabe\""));
        assert!(text.contains("\"reply\":{"));
        assert!(text.contains("\"mentions\":[\"alice-pub\"]"));
        assert!(text.contains("\"pubkey\":"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn loads_recent_and_resolves_short_id() {
        let dir = std::env::temp_dir().join(format!("asa-history-load-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let history = History::new(dir.clone(), "abc".to_string()).unwrap();
        let pk = "a".repeat(64);
        history.append_chat_at("alice", "aaaaaaaa", &pk, "one", "id1", 1, None, &[]);
        history.append_chat("bob", "bbbbbbbb", &"b".repeat(64), "two", "id2", None, &[]);
        let all = history.load_recent(0, |_| None);
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].text, "one");
        assert_eq!(all[1].text, "two");
        let recent = history.load_recent(1, |_| None);
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].text, "two");
        // 旧日志没有 pubkey 时，可以通过短 ID 查回。
        let resolved = history.load_recent(2, |short| {
            if short == "aaaaaaaa" {
                Some(pk.clone())
            } else {
                None
            }
        });
        assert!(resolved
            .iter()
            .any(|chat| chat.pubkey.as_deref() == Some(pk.as_str())));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

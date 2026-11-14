//! 本地聊天记录：按日期追加写 JSON Lines。
//!
//! 遵循「仅本地记录」策略：不向新成员补发历史。

use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Serialize;

use crate::proto::ReplyRef;
use crate::util::now_ms;

#[derive(Serialize)]
struct Record<'a> {
    ts: u64,
    kind: &'a str,
    from: &'a str,
    from_id: &'a str,
    text: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reply: Option<&'a ReplyRef>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    mentions: Vec<String>,
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
            text,
            id: None,
            reply: None,
            mentions: Vec::new(),
        });
    }

    /// 追加一条聊天记录，携带消息 id、回复引用和提及列表。
    pub fn append_chat(
        &self,
        from: &str,
        from_id: &str,
        text: &str,
        id: &str,
        reply: Option<&ReplyRef>,
        mentions: &[String],
    ) {
        self.write(Record {
            ts: now_ms(),
            kind: "chat",
            from,
            from_id,
            text,
            id: Some(id),
            reply,
            mentions: mentions.to_vec(),
        });
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
        history.append_chat(
            "bob",
            "b0b",
            "收到",
            "cafebabe",
            Some(&reply),
            &["alice-pub".to_string()],
        );
        let text = std::fs::read_to_string(history.current_path()).unwrap();
        assert!(text.contains("\"id\":\"cafebabe\""));
        assert!(text.contains("\"reply\":{"));
        assert!(text.contains("\"mentions\":[\"alice-pub\"]"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

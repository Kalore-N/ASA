//! 通用小工具：时间、ID、文件大小、文件名处理等。

use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};

/// 当前 Unix 毫秒时间戳。
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 本机在局域网中最可能可用的 IPv4 地址。
///
/// 通过 UDP `connect` 让内核挑选出口网卡（不会真的发包），
/// 失败时回退到 `127.0.0.1`。
pub fn lan_ipv4() -> IpAddr {
    let fallback = IpAddr::V4(Ipv4Addr::LOCALHOST);
    let Ok(sock) = UdpSocket::bind("0.0.0.0:0") else {
        return fallback;
    };
    if sock.connect("8.8.8.8:80").is_err() {
        return fallback;
    }
    sock.local_addr().map(|a| a.ip()).unwrap_or(fallback)
}

/// 对公钥做哈希，得到稳定的短 ID（用于展示与匹配）。
pub fn peer_id(pubkey_hex: &str) -> String {
    let digest = Sha256::digest(pubkey_hex.as_bytes());
    hex::encode(&digest[..8])
}

/// 短 ID 的前 8 个字符，界面展示用。
pub fn short_id(pubkey_hex: &str) -> String {
    peer_id(pubkey_hex).chars().take(8).collect()
}

/// 人类可读的文件大小。
pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// 本地时区的 HH:MM。
pub fn hhmm(ts_ms: u64) -> String {
    use chrono::TimeZone;
    chrono::Local
        .timestamp_millis_opt(ts_ms as i64)
        .single()
        .map(|dt| dt.format("%H:%M").to_string())
        .unwrap_or_else(|| "--:--".to_string())
}

/// 本地时区的 HH:MM:SS。
pub fn hhmmss(ts_ms: u64) -> String {
    use chrono::TimeZone;
    chrono::Local
        .timestamp_millis_opt(ts_ms as i64)
        .single()
        .map(|dt| dt.format("%H:%M:%S").to_string())
        .unwrap_or_else(|| "--:--:--".to_string())
}

/// 展开 `~` 前缀。
pub fn expand_tilde(path: &str) -> PathBuf {
    let trimmed = path.trim().trim_matches('"').trim_matches('\'');
    if trimmed == "~" {
        return crate::paths::home_dir().unwrap_or_else(|_| PathBuf::from(trimmed));
    }
    if let Some(rest) = trimmed
        .strip_prefix("~/")
        .or_else(|| trimmed.strip_prefix("~\\"))
    {
        if let Ok(home) = crate::paths::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(trimmed)
}

/// 只保留文件名本身，去掉路径分隔符与控制字符。
pub fn sanitize_filename(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or("file");
    let cleaned: String = base
        .chars()
        .filter(|c| !c.is_control() && *c != ':' && *c != '*' && *c != '?' && *c != '"')
        .collect();
    let trimmed = cleaned.trim().trim_matches('.').trim();
    if trimmed.is_empty() {
        "file".to_string()
    } else {
        trimmed.chars().take(180).collect()
    }
}

/// 在目录下选一个不冲突的文件名。
pub fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let candidate = dir.join(name);
    if !candidate.exists() {
        return candidate;
    }
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s.to_string(), format!(".{e}")),
        _ => (name.to_string(), String::new()),
    };
    for i in 1..10_000u32 {
        let candidate = dir.join(format!("{stem} ({i}){ext}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    dir.join(format!("{}-{name}", now_ms()))
}

/// 把字符串里的控制字符替换成空格，压掉多余空白（保留换行）。
pub fn sanitize_text(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        if ch == '\n' || ch == '\t' {
            out.push(' ');
        } else if ch.is_control() {
            continue;
        } else {
            out.push(ch);
        }
    }
    out.trim().to_string()
}

/// 从字符串里挑一个稳定的颜色索引（用于给昵称上色）。
pub fn color_index(seed: &str) -> usize {
    seed.bytes().fold(0usize, |acc, b| {
        acc.wrapping_mul(31).wrapping_add(b as usize)
    }) % 7
}

/// 把地址转成可展示的字符串。
pub fn addr_string(addr: Option<SocketAddr>) -> String {
    addr.map(|a| a.to_string())
        .unwrap_or_else(|| "-".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_are_readable() {
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(1024), "1.0 KiB");
        assert_eq!(human_size(1024 * 1024 * 3 / 2), "1.5 MiB");
    }

    #[test]
    fn filenames_are_sanitized() {
        assert_eq!(sanitize_filename("../../etc/passwd"), "passwd");
        assert_eq!(sanitize_filename("a:b.txt"), "ab.txt");
        assert_eq!(sanitize_filename("   "), "file");
    }

    #[test]
    fn peer_ids_are_stable() {
        assert_eq!(peer_id("abc").len(), 16);
        assert_eq!(short_id("abc").len(), 8);
        assert_eq!(peer_id("abc"), peer_id("abc"));
    }
}

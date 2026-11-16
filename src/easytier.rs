//! EasyTier 组网层。
//!
//! ASA 不把 EasyTier 当作应用层中继，而是把它当作可选的虚拟局域网：
//! - 同一频道的成员用频道密钥派生同一个 EasyTier 网络名/密钥；
//! - 每个成员用公钥派生固定的 `10.144.x.y/16` 虚拟 IP；
//! - 邀请码只携带公共节点地址，加入方据此找到同一个 EasyTier 网络。
//!
//! 桌面端通过随包附带的 `easytier-core` 子进程运行；Android 端通过
//! `libeasytier_android_jni.so` + VpnService 运行。两边生成同一份 TOML。

use std::io::{Read, Seek, SeekFrom};
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use sha2::{Digest, Sha256};

use crate::crypto::sha256_hex;
use crate::paths::RuntimeDirs;
use crate::room::RoomConfig;

/// EasyTier 虚拟网段。所有 ASA 节点使用同一个 /16，避免小网段地址冲突。
pub const ET_PREFIX: u8 = 16;

/// 校验并规范化一个 EasyTier 节点地址。
///
/// 支持：
/// - `tcp://host:11010`、`udp://host:11010`
/// - `ws://host`、`wss://host`（可省略端口，默认 80/443）
/// - 裸 `host:11010`（按 tcp 处理）
///
/// 打码地址（含 `*`）会直接拒绝，并提示去 EasyTier QQ 群获取完整地址。
pub fn normalize_node(text: &str) -> Result<String> {
    let text = text.trim();
    if text.is_empty() {
        bail!("节点地址为空");
    }
    if text.contains('*') {
        bail!("该节点地址被打码，请从 EasyTier QQ 群获取完整地址");
    }
    if text.chars().any(|ch| ch.is_control() || ch.is_whitespace()) {
        bail!("节点地址不能包含空白或控制字符");
    }

    let (scheme, rest) = match text.split_once("://") {
        Some((scheme, rest)) => (scheme.to_ascii_lowercase(), rest),
        None => ("tcp".to_string(), text),
    };
    if !matches!(
        scheme.as_str(),
        "tcp" | "udp" | "ws" | "wss" | "quic" | "wg" | "faketcp"
    ) {
        bail!("不支持的节点协议: {scheme}");
    }

    let rest = rest.trim_end_matches('/');
    if rest.is_empty() {
        bail!("节点地址缺少主机名");
    }

    // IPv6 形如 [::1]:11010；普通域名/IPv4 形如 host:11010。
    let (host, port_text) = if let Some(after) = rest.strip_prefix('[') {
        let (host, port) = after
            .split_once("]:")
            .ok_or_else(|| anyhow!("IPv6 节点地址缺少端口，示例：tcp://[::1]:11010"))?;
        (host.to_string(), Some(port))
    } else if let Some((host, port)) = rest.rsplit_once(':') {
        (host.to_string(), Some(port))
    } else {
        (rest.to_string(), None)
    };
    if host.is_empty() {
        bail!("节点主机名为空");
    }
    if host.len() > 253 {
        bail!("节点主机名过长");
    }

    let port = match port_text {
        Some(port) => {
            let port: u16 = port.parse().context("节点端口无效")?;
            if port == 0 {
                bail!("节点端口不能为 0");
            }
            Some(port)
        }
        None => match scheme.as_str() {
            "ws" => Some(80),
            "wss" => Some(443),
            _ => bail!("节点地址缺少端口，示例：tcp://host:11010"),
        },
    };

    let host = if host.contains(':') {
        format!("[{host}]")
    } else {
        host
    };
    Ok(format!("{scheme}://{host}:{}", port.unwrap()))
}

/// 规范化并去重一组节点地址，忽略无效项。
pub fn normalize_nodes<'a, I>(nodes: I) -> Vec<String>
where
    I: IntoIterator<Item = &'a String>,
{
    let mut out = Vec::new();
    for node in nodes {
        if let Ok(node) = normalize_node(node) {
            if !out.contains(&node) {
                out.push(node);
            }
        }
    }
    out
}

/// 把 `extra` 中合法的节点并入节点池；返回是否有新增。
pub fn merge_pool(pool: &mut Vec<String>, extra: &[String]) -> bool {
    let mut changed = false;
    for node in extra {
        if let Ok(node) = normalize_node(node) {
            if !pool.contains(&node) {
                pool.push(node);
                changed = true;
            }
        }
    }
    changed
}

/// 当前频道实际使用的节点：优先 `preferred`，否则用池中第一个。
pub fn selected_node(pool: &[String], preferred: Option<&str>) -> Option<String> {
    let preferred = preferred.and_then(|node| normalize_node(node).ok());
    preferred
        .filter(|node| pool.iter().any(|item| item == node))
        .or_else(|| pool.first().cloned())
}

/// 一个节点的连通性探测结果。
#[derive(Clone, Debug)]
pub struct NodeProbe {
    pub ok: bool,
    pub latency_ms: Option<u128>,
    pub detail: String,
}

/// 用 TCP 连接测试节点是否可达（tcp/ws/wss/faketcp）。
///
/// udp/quic 无法用 TCP 握手判断，这里只做 DNS 解析检查。
pub fn probe_node(node: &str, timeout: Duration) -> NodeProbe {
    use std::net::{TcpStream, ToSocketAddrs};

    let normalized = match normalize_node(node) {
        Ok(node) => node,
        Err(err) => {
            return NodeProbe {
                ok: false,
                latency_ms: None,
                detail: err.to_string(),
            }
        }
    };
    let (scheme, authority) = normalized
        .split_once("://")
        .map(|(scheme, rest)| (scheme, rest.trim_end_matches('/')))
        .unwrap_or(("tcp", normalized.as_str()));

    let tcp_probe = matches!(scheme, "tcp" | "ws" | "wss" | "faketcp");
    let addrs: Vec<std::net::SocketAddr> = match authority.to_socket_addrs() {
        Ok(addrs) => addrs.collect(),
        Err(err) => {
            return NodeProbe {
                ok: false,
                latency_ms: None,
                detail: format!("DNS 解析失败: {err}"),
            }
        }
    };
    if addrs.is_empty() {
        return NodeProbe {
            ok: false,
            latency_ms: None,
            detail: "没有解析到地址".to_string(),
        };
    }
    if !tcp_probe {
        return NodeProbe {
            ok: true,
            latency_ms: None,
            detail: format!("{scheme} 节点：DNS 解析正常（TCP 探测不适用）"),
        };
    }
    let mut last_error = String::new();
    for addr in addrs {
        let started = Instant::now();
        match TcpStream::connect_timeout(&addr, timeout) {
            Ok(_) => {
                return NodeProbe {
                    ok: true,
                    latency_ms: Some(started.elapsed().as_millis()),
                    detail: format!("TCP 连接成功（{addr}）"),
                }
            }
            Err(err) => last_error = format!("{addr}: {err}"),
        }
    }
    NodeProbe {
        ok: false,
        latency_ms: None,
        detail: last_error,
    }
}

/// 从频道密钥派生 EasyTier 网络名与密钥。
///
/// 邀请码里已经包含频道密钥，所以加入方不需要额外传递 EasyTier 配置；
/// 只需知道用哪个公共节点。
pub fn derive_network(key: &[u8; 32]) -> (String, String) {
    let mut name_input = b"asa-et-name".to_vec();
    name_input.extend_from_slice(key);
    let name = sha256_hex(&name_input);
    let mut secret_input = b"asa-et-secret".to_vec();
    secret_input.extend_from_slice(key);
    let secret = sha256_hex(&secret_input);
    (format!("asa-{}", &name[..16]), secret)
}

/// 从 ASA 公钥派生固定的 EasyTier IPv4 地址。
///
/// 使用 `10.144.0.0/16`，给每个成员约 1/65536 的碰撞概率；
/// 即使碰撞，邀请码里的主地址是实际使用的地址，加入方仍能连接。
pub fn derive_ipv4(pubkey_hex: &str) -> Ipv4Addr {
    let digest = Sha256::digest(format!("asa-et-ip:{pubkey_hex}").as_bytes());
    let mut a = digest[0];
    let mut b = digest[1];
    if a == 0 {
        a = 1;
    }
    if a == 255 && b == 255 {
        b = 254;
    }
    Ipv4Addr::new(10, 144, a, b)
}

/// 生成 EasyTier 的 TOML 配置。
///
/// 注意：必须使用 `[network_identity]` 和 `[[peer]]` 的嵌套结构。
/// 顶层 `network_name` / `peers = [...]` 会被 EasyTier 静默忽略。
pub fn config_toml(
    instance: &str,
    network: &str,
    secret: &str,
    ipv4: Ipv4Addr,
    nodes: &[String],
) -> String {
    let mut text = String::new();
    text.push_str(&format!("instance_name = \"{}\"\n", toml_escape(instance)));
    text.push_str(&format!("ipv4 = \"{ipv4}/{ET_PREFIX}\"\n"));
    text.push_str("listeners = [\"tcp://0.0.0.0:11010\", \"udp://0.0.0.0:11010\"]\n");
    text.push_str("mtu = 1380\n\n");
    text.push_str("[network_identity]\n");
    text.push_str(&format!("network_name = \"{}\"\n", toml_escape(network)));
    text.push_str(&format!("network_secret = \"{}\"\n", toml_escape(secret)));
    for node in nodes {
        text.push_str("\n[[peer]]\n");
        text.push_str(&format!("uri = \"{}\"\n", toml_escape(node)));
    }
    text
}

fn toml_escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

/// 供 TUI/Android 展示的 EasyTier 状态快照。
#[derive(Clone, Debug, Default)]
pub struct EtStatus {
    pub running: bool,
    pub ip: Option<Ipv4Addr>,
    pub nodes: Vec<String>,
    pub error: Option<String>,
}

/// 一次 EasyTier 运行会话。
pub struct EtProcess {
    child: Child,
    pub ip: Ipv4Addr,
    pub nodes: Vec<String>,
    pub config_path: PathBuf,
    pub log_path: PathBuf,
}

impl EtProcess {
    /// 启动 EasyTier sidecar，并等待 TUN 就绪。
    pub fn start(
        dirs: &RuntimeDirs,
        room: &RoomConfig,
        pubkey: &str,
        nodes: &[String],
    ) -> Result<Self> {
        if nodes.is_empty() {
            bail!("没有可用的 EasyTier 节点");
        }
        let core = find_core()?;
        let (network, secret) = derive_network(&room.key);
        let ip = derive_ipv4(pubkey);
        let instance = format!("asa-{}", room.id);

        let et_dir = dirs.data.join("easytier");
        std::fs::create_dir_all(&et_dir)
            .with_context(|| format!("创建 EasyTier 目录失败: {}", et_dir.display()))?;
        let config_path = et_dir.join(format!("{}.toml", room.id));
        let log_path = dirs.logs.join(format!("easytier-{}.log", room.id));
        let toml = config_toml(&instance, &network, &secret, ip, nodes);
        write_private(&config_path, &toml)?;

        let stdout = std::fs::File::create(&log_path)
            .with_context(|| format!("创建 EasyTier 日志失败: {}", log_path.display()))?;
        let stderr = stdout.try_clone().context("复制 EasyTier 日志句柄失败")?;

        let mut child = Command::new(&core)
            .arg("--config-file")
            .arg(&config_path)
            .stdin(Stdio::null())
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr))
            .spawn()
            .with_context(|| format!("启动 EasyTier 失败: {}", core.display()))?;

        // 等待 TUN 就绪；固定 IP 通常几百毫秒内完成。
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if let Some(status) = child.try_wait().context("等待 EasyTier 退出失败")? {
                let log_text = read_tail(&log_path, 4000);
                bail!(
                    "EasyTier 启动失败（{status}）：{}{}",
                    log_text.trim(),
                    privilege_hint(&core)
                );
            }
            let log_text = read_tail(&log_path, 8000);
            if log_text.contains("tun device ready") || log_text.contains("TUN device ready") {
                break;
            }
            if log_text.contains("Operation not permitted")
                || log_text.contains("Permission denied")
                || log_text.contains("failed to create tun")
            {
                let _ = child.kill();
                let _ = child.wait();
                bail!(
                    "EasyTier 无法创建 TUN 设备：{}{}",
                    log_text.trim(),
                    privilege_hint(&core)
                );
            }
            std::thread::sleep(Duration::from_millis(120));
        }

        if let Some(status) = child.try_wait().context("等待 EasyTier 退出失败")? {
            let log_text = read_tail(&log_path, 4000);
            bail!(
                "EasyTier 启动后立即退出（{status}）：{}{}",
                log_text.trim(),
                privilege_hint(&core)
            );
        }

        Ok(Self {
            child,
            ip,
            nodes: nodes.to_vec(),
            config_path,
            log_path,
        })
    }

    pub fn stop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    pub fn log_tail(&self, limit: usize) -> String {
        read_tail(&self.log_path, limit)
    }
}

impl Drop for EtProcess {
    fn drop(&mut self) {
        self.stop();
    }
}

fn privilege_hint(core: &Path) -> String {
    #[cfg(target_os = "linux")]
    {
        format!(
            "\nLinux 需要 TUN 权限：sudo setcap cap_net_admin,cap_net_raw+ep {}",
            core.display()
        )
    }
    #[cfg(target_os = "windows")]
    {
        let _ = core;
        "\nWindows 需要以管理员身份运行 ASA".to_string()
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    {
        let _ = core;
        String::new()
    }
}

/// 查找随包附带或 PATH 中的 `easytier-core`。
pub fn find_core() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("ASA_EASYTIER_CORE") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Ok(path);
        }
        bail!("ASA_EASYTIER_CORE 指向的文件不存在: {}", path.display());
    }

    let name = if cfg!(windows) {
        "easytier-core.exe"
    } else {
        "easytier-core"
    };

    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            for candidate in [dir.join(name), dir.join("easytier").join(name)] {
                if candidate.is_file() {
                    return Ok(candidate);
                }
            }
        }
    }

    #[cfg(unix)]
    {
        let candidate = PathBuf::from("/usr/lib/asa").join(name);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }

    if let Some(paths) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&paths) {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }

    bail!("找不到 {name}。请把 EasyTier 随 ASA 一起安装，或设置 ASA_EASYTIER_CORE 指向它")
}

fn write_private(path: &Path, text: &str) -> Result<()> {
    std::fs::write(path, text)
        .with_context(|| format!("写入 EasyTier 配置失败: {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

fn read_tail(path: &Path, limit: usize) -> String {
    let Ok(mut file) = std::fs::File::open(path) else {
        return String::new();
    };
    let len = file.metadata().map(|m| m.len()).unwrap_or(0);
    let start = len.saturating_sub(limit as u64);
    if file.seek(SeekFrom::Start(start)).is_err() {
        return String::new();
    }
    let mut buf = Vec::new();
    let _ = file.read_to_end(&mut buf);
    String::from_utf8_lossy(&buf).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_tcp_and_bare() {
        assert_eq!(
            normalize_node("tcp://a.example:11010").unwrap(),
            "tcp://a.example:11010"
        );
        assert_eq!(
            normalize_node("a.example:11010").unwrap(),
            "tcp://a.example:11010"
        );
        assert_eq!(
            normalize_node("wss://et.example").unwrap(),
            "wss://et.example:443"
        );
    }

    #[test]
    fn normalize_rejects_masked_and_bad() {
        assert!(normalize_node("tcp://et1.f*s.top:11010").is_err());
        assert!(normalize_node("tcp://host").is_err());
        assert!(normalize_node("http://host:80").is_err());
        assert!(normalize_node("tcp://host:0").is_err());
    }

    #[test]
    fn ipv4_is_stable_and_in_prefix() {
        let a = derive_ipv4("aa");
        let b = derive_ipv4("aa");
        let c = derive_ipv4("bb");
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert!(a.octets()[0] == 10 && a.octets()[1] == 144);
    }

    #[test]
    fn network_is_stable() {
        let key = [7u8; 32];
        let (name1, secret1) = derive_network(&key);
        let (name2, secret2) = derive_network(&key);
        assert_eq!(name1, name2);
        assert_eq!(secret1, secret2);
        assert!(name1.starts_with("asa-"));
        assert_eq!(secret1.len(), 64);
    }

    #[test]
    fn toml_uses_nested_identity_and_peer() {
        let text = config_toml(
            "asa-test",
            "asa-net",
            "secret",
            Ipv4Addr::new(10, 144, 1, 2),
            &["tcp://node.example:11010".to_string()],
        );
        assert!(text.contains("[network_identity]"));
        assert!(text.contains("network_name = \"asa-net\""));
        assert!(text.contains("[[peer]]"));
        assert!(text.contains("uri = \"tcp://node.example:11010\""));
        assert!(!text.contains("\npeers ="));
    }

    #[test]
    fn toml_escapes_quotes() {
        let text = config_toml("a\"b", "n\\e", "s\"x", Ipv4Addr::new(10, 144, 1, 2), &[]);
        assert!(text.contains("instance_name = \"a\\\"b\""));
        assert!(text.contains("network_name = \"n\\\\e\""));
    }
}

#[cfg(test)]
mod node_tests {
    use super::*;

    #[test]
    fn normalizes_and_dedups_nodes() {
        let nodes = vec![
            "tcp://a.example:11010/".to_string(),
            "tcp://a.example:11010".to_string(),
            "bad node".to_string(),
            "udp://b.example:11010".to_string(),
        ];
        let pool = normalize_nodes(nodes.iter());
        assert_eq!(
            pool,
            vec![
                "tcp://a.example:11010".to_string(),
                "udp://b.example:11010".to_string()
            ]
        );
    }

    #[test]
    fn selected_node_prefers_valid_channel_choice() {
        let pool = vec![
            "tcp://a.example:11010".to_string(),
            "tcp://b.example:11010".to_string(),
        ];
        assert_eq!(
            selected_node(&pool, Some("tcp://b.example:11010")).as_deref(),
            Some("tcp://b.example:11010")
        );
        assert_eq!(
            selected_node(&pool, Some("tcp://missing.example:11010")).as_deref(),
            Some("tcp://a.example:11010")
        );
        assert_eq!(selected_node(&[], None), None);
    }

    #[test]
    fn probes_a_local_tcp_listener() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let accept = std::thread::spawn(move || {
            let _ = listener.accept();
        });
        let probe = probe_node(&format!("tcp://127.0.0.1:{port}"), Duration::from_secs(2));
        assert!(probe.ok, "{}", probe.detail);
        assert!(probe.latency_ms.is_some());
        accept.join().unwrap();
    }
}

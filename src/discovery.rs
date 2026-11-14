//! 局域网自动发现（mDNS，服务类型 `_asa._tcp.local.`）。
//!
//! 只在同一局域网内生效；跨网需要手动 `--connect` 或邀请码里的地址。

use std::net::SocketAddr;
use std::sync::mpsc::Sender;
use std::sync::{Arc, RwLock};

use anyhow::{Context, Result};
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};

const SERVICE_TYPE: &str = "_asa._tcp.local.";

#[derive(Clone, Debug)]
pub struct Discovered {
    pub room: String,
    pub peer: String,
    pub nick: String,
    pub addr: SocketAddr,
}

pub struct Discovery {
    daemon: ServiceDaemon,
    room: String,
    peer: String,
    port: u16,
    nick: Arc<RwLock<String>>,
}

impl Discovery {
    pub fn start(
        room: &str,
        peer: &str,
        nick: Arc<RwLock<String>>,
        port: u16,
        out: Sender<Discovered>,
    ) -> Result<Self> {
        let daemon = ServiceDaemon::new().context("创建 mDNS 服务失败")?;
        let discovery = Discovery {
            daemon,
            room: room.to_string(),
            peer: peer.to_string(),
            port,
            nick,
        };
        discovery.register()?;
        discovery.browse(out)?;
        Ok(discovery)
    }

    fn register(&self) -> Result<()> {
        let suffix: String = self.peer.chars().take(12).collect();
        let instance = format!("asa-{suffix}");
        let hostname = format!("asa-{suffix}.local.");
        let nick = self.nick.read().map(|n| n.clone()).unwrap_or_default();
        let properties = vec![
            ("room".to_string(), self.room.clone()),
            ("peer".to_string(), self.peer.clone()),
            ("nick".to_string(), nick),
            ("v".to_string(), "1".to_string()),
        ];
        let info = ServiceInfo::new(
            SERVICE_TYPE,
            &instance,
            &hostname,
            "",
            self.port,
            properties.as_slice(),
        )
        .context("构造 mDNS 服务信息失败")?
        .enable_addr_auto();
        self.daemon.register(info).context("注册 mDNS 服务失败")?;
        Ok(())
    }

    /// 昵称变化后刷新 TXT 记录。
    pub fn refresh(&self) {
        let _ = self.register();
    }

    fn browse(&self, out: Sender<Discovered>) -> Result<()> {
        let receiver = self
            .daemon
            .browse(SERVICE_TYPE)
            .context("启动 mDNS 浏览失败")?;
        let room = self.room.clone();
        let peer = self.peer.clone();
        std::thread::spawn(move || {
            while let Ok(event) = receiver.recv() {
                let ServiceEvent::ServiceResolved(info) = event else {
                    continue;
                };
                if info.get_property_val_str("room") != Some(room.as_str()) {
                    continue;
                }
                let their_peer = info.get_property_val_str("peer").unwrap_or("");
                if their_peer.is_empty() || their_peer == peer {
                    continue;
                }
                let nick = info.get_property_val_str("nick").unwrap_or("").to_string();
                let port = info.get_port();
                for scoped in info.get_addresses() {
                    // 只走 IPv4，避免部分系统 IPv6 可达性差异带来的连接失败。
                    if !scoped.is_ipv4() {
                        continue;
                    }
                    let addr = SocketAddr::new(scoped.to_ip_addr(), port);
                    let _ = out.send(Discovered {
                        room: room.clone(),
                        peer: their_peer.to_string(),
                        nick: nick.clone(),
                        addr,
                    });
                }
            }
        });
        Ok(())
    }
}

impl Drop for Discovery {
    fn drop(&mut self) {
        let _ = self.daemon.shutdown();
    }
}

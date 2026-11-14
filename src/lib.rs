//! ASA —— 去中心化 TUI 群聊。
//!
//! 所有节点对等：任何人都能接受新成员，消息在成员间泛洪传播，
//! 「服主」只是一个由在场时间决定的逻辑角色，用于成员表同步与新成员入口。

pub mod config;
pub mod crypto;
pub mod discovery;
pub mod history;
pub mod identity;
pub mod net;
pub mod node;
pub mod paths;
pub mod proto;
pub mod relay;
pub mod room;
pub mod settings;
pub mod tui;
pub mod util;

#[cfg(target_os = "android")]
pub mod android;

pub use node::{Cmd, Node, NodeConfig, NodeEvent, NodeHandle};

//! 线路协议：帧格式、握手消息、加密信封与业务载荷。
//!
//! 传输层是长度前缀（u32 大端）+ JSON，便于跨平台调试。

use anyhow::{anyhow, bail, Context, Result};
use ed25519_dalek::Signer;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt};

use crate::crypto::{self, RoomKey};

pub const PROTO_V: u8 = 1;
pub const MAX_FRAME: usize = 16 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum Frame {
    Hello(Hello),
    /// 握手第三步：证明自己持有 Hello 里声明的私钥（防重放/冒名）。
    Proof {
        sig: String,
    },
    Bye {
        reason: String,
    },
    Envelope(Envelope),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Hello {
    pub v: u8,
    pub room: String,
    pub pubkey: String,
    pub nick: String,
    pub listen: String,
    /// 本节点加入群的时间（毫秒），用于服主排序。
    pub presence: u64,
    /// 本次握手的随机数（base64）。
    pub nonce: String,
    /// `hello_sig_bytes` 的签名。
    pub sig: String,
    /// 用群密钥加密的 nonce，证明自己持有群密钥。
    pub auth: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Envelope {
    pub id: String,
    pub room: String,
    pub sender: String,
    pub ts: u64,
    pub nonce: String,
    pub ct: String,
    pub sig: String,
}

/// 回复引用：记录被回复消息的标识与摘要，便于在未同步历史的节点上展示。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplyRef {
    /// 被回复消息的信封 id。
    pub id: String,
    /// 被回复消息的发送者昵称。
    pub nick: String,
    /// 被回复消息的文本摘要（已截断）。
    pub preview: String,
}

/// 从其他成员同步过来的聊天记录。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncMessage {
    pub id: String,
    pub from: String,
    pub nick: String,
    pub text: String,
    pub ts: u64,
    #[serde(default)]
    pub mentions: Vec<String>,
    #[serde(default)]
    pub reply: Option<ReplyRef>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "k", rename_all = "snake_case")]
pub enum Payload {
    Chat {
        text: String,
        /// 被 @ 的成员公钥；由发送端解析，避免昵称歧义。
        #[serde(default)]
        mentions: Vec<String>,
        /// 回复引用。
        #[serde(default)]
        reply: Option<ReplyRef>,
    },
    /// 请求最近聊天记录。
    SyncRequest {
        limit: u32,
    },
    /// 回复最近聊天记录。
    SyncBatch {
        messages: Vec<SyncMessage>,
    },
    /// 在线心跳，携带昵称/地址/在场时间。
    Presence {
        nick: String,
        listen: String,
        presence: u64,
    },
    /// 服主广播的成员表（用于 PEX 与断线重连）。
    Members {
        members: Vec<MemberInfo>,
    },
    FileOffer {
        xfer: String,
        name: String,
        size: u64,
        sha256: String,
    },
    FileAccept {
        xfer: String,
    },
    FileReject {
        xfer: String,
        reason: String,
    },
    FileChunk {
        xfer: String,
        idx: u32,
        total: u32,
        data: String,
    },
    FileDone {
        xfer: String,
    },
    FileResult {
        xfer: String,
        ok: bool,
        msg: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MemberInfo {
    pub pubkey: String,
    pub nick: String,
    #[serde(default)]
    pub listen: String,
    pub presence: u64,
}

/// 握手 Hello 的签名原文。
pub fn hello_sig_bytes(h: &Hello) -> Vec<u8> {
    format!(
        "asa1|hello|{}|{}|{}|{}|{}|{}|{}",
        h.v, h.room, h.pubkey, h.nick, h.listen, h.presence, h.nonce
    )
    .into_bytes()
}

/// 握手第三步的签名原文：把双方 nonce 与角色绑定，抵御重放与 MITM。
pub fn proof_bytes(connector: &str, acceptor: &str, nonce_c: &str, nonce_s: &str) -> Vec<u8> {
    format!("asa1|proof|{connector}|{acceptor}|{nonce_c}|{nonce_s}").into_bytes()
}

/// 信封的签名原文。
pub fn env_sig_bytes(e: &Envelope) -> Vec<u8> {
    format!(
        "asa1|env|{}|{}|{}|{}|{}|{}",
        e.id, e.room, e.sender, e.ts, e.nonce, e.ct
    )
    .into_bytes()
}

/// 载荷加密的 AAD，把房间与发送者绑定进密文。
pub fn env_aad(room: &str, sender: &str) -> Vec<u8> {
    format!("asa1|payload|{room}|{sender}").into_bytes()
}

pub fn encode_frame(frame: &Frame) -> Result<Vec<u8>> {
    let body = serde_json::to_vec(frame)?;
    if body.len() > MAX_FRAME {
        bail!("帧过大: {} 字节", body.len());
    }
    let mut out = Vec::with_capacity(body.len() + 4);
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    out.extend_from_slice(&body);
    Ok(out)
}

pub async fn read_frame<R: AsyncRead + Unpin>(reader: &mut R) -> Result<Option<Frame>> {
    let mut len_buf = [0u8; 4];
    if let Err(err) = reader.read_exact(&mut len_buf).await {
        if err.kind() == std::io::ErrorKind::UnexpectedEof {
            return Ok(None);
        }
        return Err(err.into());
    }
    let len = u32::from_be_bytes(len_buf) as usize;
    if len == 0 || len > MAX_FRAME {
        bail!("非法帧长度: {len}");
    }
    let mut body = vec![0u8; len];
    reader.read_exact(&mut body).await?;
    let frame = serde_json::from_slice(&body).context("帧 JSON 解析失败")?;
    Ok(Some(frame))
}

/// 把载荷封装成加密、签名的信封帧。
pub fn build_envelope(
    key: &RoomKey,
    signing: &ed25519_dalek::SigningKey,
    pubkey: &str,
    room: &str,
    payload: &Payload,
) -> Result<Vec<u8>> {
    Ok(build_envelope_with_id(key, signing, pubkey, room, payload)?.1)
}

/// 同 [`build_envelope`]，但额外返回信封 id（用于回复引用）。
pub fn build_envelope_with_id(
    key: &RoomKey,
    signing: &ed25519_dalek::SigningKey,
    pubkey: &str,
    room: &str,
    payload: &Payload,
) -> Result<(String, Vec<u8>)> {
    let plain = serde_json::to_vec(payload)?;
    let aad = env_aad(room, pubkey);
    let (ct, nonce) = crypto::seal(key, &aad, &plain)?;
    let mut env = Envelope {
        id: hex::encode(crypto::random_bytes::<8>()),
        room: room.to_string(),
        sender: pubkey.to_string(),
        ts: crate::util::now_ms(),
        nonce: crypto::b64e(&nonce),
        ct: crypto::b64e(&ct),
        sig: String::new(),
    };
    let sig = signing.sign(&env_sig_bytes(&env));
    env.sig = crypto::b64e(&sig.to_bytes());
    let id = env.id.clone();
    Ok((id, encode_frame(&Frame::Envelope(env))?))
}

/// 校验并解密信封，返回发送者公钥与载荷。
pub fn open_envelope(key: &RoomKey, room: &str, env: &Envelope) -> Result<Payload> {
    if env.room != room {
        bail!("信封属于其他群");
    }
    let sig = crypto::b64d(&env.sig).context("签名格式错误")?;
    let sig_arr: [u8; 64] = sig
        .as_slice()
        .try_into()
        .map_err(|_| anyhow!("签名长度错误"))?;
    crate::identity::verify_b64(&env.sender, &env_sig_bytes(env), &crypto::b64e(&sig_arr))?;
    let nonce_raw = crypto::b64d(&env.nonce).context("nonce 格式错误")?;
    let nonce: [u8; 12] = nonce_raw
        .as_slice()
        .try_into()
        .map_err(|_| anyhow!("nonce 长度错误"))?;
    let ct = crypto::b64d(&env.ct).context("密文格式错误")?;
    let aad = env_aad(room, &env.sender);
    let plain = crypto::open(key, &aad, &nonce, &ct)?;
    let payload = serde_json::from_slice(&plain).context("载荷解析失败")?;
    Ok(payload)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::Identity;

    #[test]
    fn frame_roundtrip() {
        let frame = Frame::Proof {
            sig: "abc".to_string(),
        };
        let bytes = encode_frame(&frame).unwrap();
        assert_eq!(&bytes[..4], &(bytes.len() as u32 - 4).to_be_bytes());
    }

    #[test]
    fn envelope_roundtrip() {
        let ident = Identity::generate();
        let key = [3u8; 32];
        let payload = Payload::Chat {
            text: "你好".to_string(),
            mentions: vec!["bob-pubkey".to_string()],
            reply: Some(ReplyRef {
                id: "abc123".to_string(),
                nick: "bob".to_string(),
                preview: "原始消息".to_string(),
            }),
        };
        let (id, bytes) =
            build_envelope_with_id(&key, &ident.signing, &ident.pubkey_hex(), "room1", &payload)
                .unwrap();
        assert!(!id.is_empty());
        let body = &bytes[4..];
        let frame: Frame = serde_json::from_slice(body).unwrap();
        let Frame::Envelope(env) = frame else {
            panic!("不是信封");
        };
        assert_eq!(env.id, id);
        let opened = open_envelope(&key, "room1", &env).unwrap();
        match opened {
            Payload::Chat {
                text,
                mentions,
                reply,
            } => {
                assert_eq!(text, "你好");
                assert_eq!(mentions, vec!["bob-pubkey".to_string()]);
                let reply = reply.expect("应带回复引用");
                assert_eq!(reply.id, "abc123");
                assert_eq!(reply.nick, "bob");
                assert_eq!(reply.preview, "原始消息");
            }
            _ => panic!("载荷类型错误"),
        }
        assert!(open_envelope(&[4u8; 32], "room1", &env).is_err());
    }

    #[test]
    fn old_chat_payload_still_parses() {
        // 旧版节点只发送 text，必须能向后兼容。
        let payload: Payload = serde_json::from_str(r#"{"k":"chat","text":"hi"}"#).unwrap();
        match payload {
            Payload::Chat {
                text,
                mentions,
                reply,
            } => {
                assert_eq!(text, "hi");
                assert!(mentions.is_empty());
                assert!(reply.is_none());
            }
            _ => panic!("载荷类型错误"),
        }
    }

    #[test]
    fn sync_batch_roundtrip() {
        let payload = Payload::SyncBatch {
            messages: vec![SyncMessage {
                id: "id1".to_string(),
                from: "pk".to_string(),
                nick: "alice".to_string(),
                text: "hello".to_string(),
                ts: 123,
                mentions: vec![],
                reply: None,
            }],
        };
        let text = serde_json::to_string(&payload).unwrap();
        let parsed: Payload = serde_json::from_str(&text).unwrap();
        match parsed {
            Payload::SyncBatch { messages } => {
                assert_eq!(messages.len(), 1);
                assert_eq!(messages[0].text, "hello");
            }
            _ => panic!("类型错误"),
        }
    }
}

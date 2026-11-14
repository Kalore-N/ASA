//! TCP 传输层与三向握手。
//!
//! 握手流程（连接方 / 接受方）：
//! 1. 连接方发 Hello（含群密钥证明与签名）
//! 2. 接受方校验后回自己的 Hello
//! 3. 双方互相发送 Proof，签名绑定双方 nonce，抵御重放与冒名。

use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use ed25519_dalek::SigningKey;
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;

use crate::crypto::{self, RoomKey};
use crate::identity::verify_b64;
use crate::proto::{self, hello_sig_bytes, proof_bytes, Frame, Hello, PROTO_V};

static CONN_SEQ: AtomicU64 = AtomicU64::new(1);

const AUTH_PLAINTEXT: &[u8] = b"asa1|auth-ok";

/// 握手与心跳所需的共享上下文。
pub struct HandshakeCtx {
    pub room: String,
    pub key: RoomKey,
    pub signing: SigningKey,
    pub pubkey: String,
    pub nick: Arc<RwLock<String>>,
    /// 对外通告的监听地址。
    pub listen: SocketAddr,
    /// 本节点加入群的时间（毫秒）。
    pub self_since: u64,
}

#[derive(Debug)]
pub enum NetEvent {
    Up {
        conn: u64,
        peer: String,
        hello: Hello,
        tx: mpsc::Sender<Vec<u8>>,
        inbound: bool,
        addr: SocketAddr,
    },
    Frame {
        peer: String,
        frame: Frame,
    },
    Down {
        conn: u64,
        peer: String,
        reason: String,
    },
    DialFailed {
        addr: SocketAddr,
        reason: String,
    },
    /// 中继返回的同群在线节点。
    RelayPeers(Vec<crate::relay::RelayPeer>),
    Notice(String),
}

pub async fn accept_loop(
    listener: TcpListener,
    ctx: Arc<HandshakeCtx>,
    events: mpsc::Sender<NetEvent>,
) {
    loop {
        match listener.accept().await {
            Ok((stream, addr)) => {
                let ctx = ctx.clone();
                let events = events.clone();
                tokio::spawn(async move {
                    if let Err(err) = handle_conn(stream, addr, ctx, events.clone(), true).await {
                        let _ = events
                            .send(NetEvent::Notice(format!("{addr} 握手失败: {err}")))
                            .await;
                    }
                });
            }
            Err(err) => {
                let _ = events
                    .send(NetEvent::Notice(format!("接受连接失败: {err}")))
                    .await;
                tokio::time::sleep(Duration::from_millis(300)).await;
            }
        }
    }
}

pub fn spawn_dial(addr: SocketAddr, ctx: Arc<HandshakeCtx>, events: mpsc::Sender<NetEvent>) {
    tokio::spawn(async move {
        let connect = tokio::time::timeout(Duration::from_secs(6), TcpStream::connect(addr)).await;
        match connect {
            Ok(Ok(stream)) => {
                if let Err(err) = handle_conn(stream, addr, ctx, events.clone(), false).await {
                    let _ = events
                        .send(NetEvent::DialFailed {
                            addr,
                            reason: err.to_string(),
                        })
                        .await;
                }
            }
            Ok(Err(err)) => {
                let _ = events
                    .send(NetEvent::DialFailed {
                        addr,
                        reason: err.to_string(),
                    })
                    .await;
            }
            Err(_) => {
                let _ = events
                    .send(NetEvent::DialFailed {
                        addr,
                        reason: "连接超时".to_string(),
                    })
                    .await;
            }
        }
    });
}

fn build_hello(ctx: &HandshakeCtx) -> Result<Hello> {
    // 用群密钥加密一段固定文本；能解开就证明对方持有群密钥。
    // 注意 nonce 必须使用 AEAD 实际使用的那一个，否则对端无法解密。
    let (auth, nonce) = crypto::seal(&ctx.key, crypto::AUTH_AAD, AUTH_PLAINTEXT)?;
    let nick = ctx
        .nick
        .read()
        .map(|n| n.clone())
        .unwrap_or_else(|_| "匿名".to_string());
    let mut hello = Hello {
        v: PROTO_V,
        room: ctx.room.clone(),
        pubkey: ctx.pubkey.clone(),
        nick,
        listen: ctx.listen.to_string(),
        presence: ctx.self_since,
        nonce: String::new(),
        sig: String::new(),
        auth: String::new(),
    };
    hello.nonce = crypto::b64e(&nonce);
    hello.auth = crypto::b64e(&auth);
    hello.sig = crate::identity::sign_b64(&ctx.signing, &hello_sig_bytes(&hello));
    Ok(hello)
}

fn verify_hello(hello: &Hello, ctx: &HandshakeCtx) -> Result<()> {
    if hello.v != PROTO_V {
        bail!("协议版本不一致: {}", hello.v);
    }
    if hello.room != ctx.room {
        bail!("群 ID 不匹配（不是同一个群）");
    }
    if hello.pubkey.len() != 64 || hex::decode(&hello.pubkey).is_err() {
        bail!("对方公钥格式错误");
    }
    if hello.nick.chars().count() > 64 {
        bail!("对方昵称过长");
    }
    verify_b64(&hello.pubkey, &hello_sig_bytes(hello), &hello.sig).context("Hello 签名校验失败")?;
    let nonce = crypto::b64d(&hello.nonce).context("nonce 格式错误")?;
    let nonce_arr: [u8; 12] = nonce
        .as_slice()
        .try_into()
        .map_err(|_| anyhow!("nonce 长度错误"))?;
    let auth = crypto::b64d(&hello.auth).context("群密钥证明格式错误")?;
    let plain = crypto::open(&ctx.key, crypto::AUTH_AAD, &nonce_arr, &auth)
        .context("群密钥不匹配：请检查邀请码")?;
    if plain != AUTH_PLAINTEXT {
        bail!("群密钥证明无效");
    }
    Ok(())
}

async fn read_hello<R: tokio::io::AsyncRead + Unpin>(reader: &mut R) -> Result<Hello> {
    match proto::read_frame(reader).await? {
        Some(Frame::Hello(hello)) => Ok(hello),
        Some(Frame::Bye { reason }) => bail!("对方断开: {reason}"),
        Some(_) => bail!("握手阶段收到意外帧"),
        None => bail!("对方在握手时断开连接"),
    }
}

async fn read_proof<R: tokio::io::AsyncRead + Unpin>(reader: &mut R) -> Result<String> {
    match proto::read_frame(reader).await? {
        Some(Frame::Proof { sig }) => Ok(sig),
        Some(Frame::Bye { reason }) => bail!("对方断开: {reason}"),
        Some(_) => bail!("握手阶段收到意外帧"),
        None => bail!("对方在握手时断开连接"),
    }
}

pub(crate) async fn handle_conn(
    stream: TcpStream,
    addr: SocketAddr,
    ctx: Arc<HandshakeCtx>,
    events: mpsc::Sender<NetEvent>,
    inbound: bool,
) -> Result<()> {
    stream.set_nodelay(true).ok();
    let (mut rd, mut wr) = stream.into_split();
    let my_hello = build_hello(&ctx)?;

    let peer_hello = tokio::time::timeout(Duration::from_secs(10), async {
        if inbound {
            let their = read_hello(&mut rd).await?;
            verify_hello(&their, &ctx)?;
            wr.write_all(&proto::encode_frame(&Frame::Hello(my_hello.clone()))?)
                .await?;
            wr.flush().await?;

            let connector = their.pubkey.clone();
            let acceptor = ctx.pubkey.clone();
            let proof_data = proof_bytes(&connector, &acceptor, &their.nonce, &my_hello.nonce);
            let their_proof = read_proof(&mut rd).await?;
            verify_b64(&connector, &proof_data, &their_proof).context("对方身份证明校验失败")?;
            let mine = crate::identity::sign_b64(&ctx.signing, &proof_data);
            wr.write_all(&proto::encode_frame(&Frame::Proof { sig: mine })?)
                .await?;
            wr.flush().await?;
            Ok::<Hello, anyhow::Error>(their)
        } else {
            wr.write_all(&proto::encode_frame(&Frame::Hello(my_hello.clone()))?)
                .await?;
            wr.flush().await?;
            let their = read_hello(&mut rd).await?;
            verify_hello(&their, &ctx)?;

            let connector = ctx.pubkey.clone();
            let acceptor = their.pubkey.clone();
            let proof_data = proof_bytes(&connector, &acceptor, &my_hello.nonce, &their.nonce);
            let mine = crate::identity::sign_b64(&ctx.signing, &proof_data);
            wr.write_all(&proto::encode_frame(&Frame::Proof { sig: mine })?)
                .await?;
            wr.flush().await?;
            let their_proof = read_proof(&mut rd).await?;
            verify_b64(&acceptor, &proof_data, &their_proof).context("对方身份证明校验失败")?;
            Ok::<Hello, anyhow::Error>(their)
        }
    })
    .await
    .context("握手超时")??;

    if peer_hello.pubkey == ctx.pubkey {
        bail!("拒绝连接到自身");
    }

    let conn = CONN_SEQ.fetch_add(1, Ordering::Relaxed);
    let peer = peer_hello.pubkey.clone();
    let (tx, mut rx) = mpsc::channel::<Vec<u8>>(256);
    events
        .send(NetEvent::Up {
            conn,
            peer: peer.clone(),
            hello: peer_hello.clone(),
            tx: tx.clone(),
            inbound,
            addr,
        })
        .await
        .map_err(|_| anyhow!("节点已关闭"))?;
    drop(tx);

    let read_loop = {
        let events = events.clone();
        let peer = peer.clone();
        async move {
            loop {
                match tokio::time::timeout(Duration::from_secs(75), proto::read_frame(&mut rd))
                    .await
                {
                    Ok(Ok(Some(frame))) => {
                        if events
                            .send(NetEvent::Frame {
                                peer: peer.clone(),
                                frame,
                            })
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                    _ => break,
                }
            }
        }
    };

    let write_loop = async move {
        while let Some(bytes) = rx.recv().await {
            if wr.write_all(&bytes).await.is_err() {
                break;
            }
        }
        let _ = wr.shutdown().await;
    };

    tokio::select! {
        _ = read_loop => {},
        _ = write_loop => {},
    }

    let _ = events
        .send(NetEvent::Down {
            conn,
            peer,
            reason: "连接关闭".to_string(),
        })
        .await;
    Ok(())
}

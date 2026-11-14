# 部署 ASA 中继

中继用于让**没有公网 IP** 的设备跨网加入群聊。它只转发已端到端加密的 TCP 字节流，
看不到聊天内容。

中继**默认无状态**：不保存群成员表、不保存群密钥，也不充当群的中心服务器。
群列表、群密钥和入口始终保存在各设备本地；任何可用中继都能参与节点发现。
建议部署多个中继，客户端会同时连接所有配置的地址，单个中继故障不会让群失联。

普通成员不需要任何配置：只要邀请码里包含中继地址（`&r=...`），加群时会自动使用。

## 方案一：任意云主机 + Docker（推荐）

需要一台有公网 IP 的 Linux 主机（1 核 1G 足够），并开放 TCP 9000 端口。

在项目根目录：

```bash
docker compose -f deploy/docker-compose.yml up -d --build
docker compose -f deploy/docker-compose.yml logs -f
```

看到：

```
ASA relay 已监听 0.0.0.0:9000
```

即成功。中继默认不保存任何群数据。

验证端口：

```bash
nc -vz <服务器公网IP> 9000
```

## 方案二：Windows / Linux 直接运行

把 `dist/asa`（Linux）或 `dist/asa-windows-x86_64.exe`（Windows）复制到服务器，运行：

```bash
./asa relay serve 9000
```

Windows：

```powershell
.\asa-windows-x86_64.exe relay serve 9000
```

需要让防火墙/安全组放行 TCP 9000。

Linux 可用 systemd：

```bash
sudo useradd --system --home /var/lib/asa --shell /usr/sbin/nologin asa
sudo mkdir -p /opt/asa
sudo cp dist/asa /opt/asa/asa
sudo cp deploy/relay.service /etc/systemd/system/asa-relay.service
sudo systemctl daemon-reload
sudo systemctl enable --now asa-relay
sudo systemctl status asa-relay
```

## 方案三：Fly.io

需要安装 `flyctl` 并登录。

```bash
# 把 deploy/fly.toml 里的 app 改成全局唯一名称
fly deploy --config deploy/fly.toml
fly ips allocate-v4
fly status
```

Fly 的 TCP 服务端口为 9000，地址形如 `<app>.fly.dev:9000`。
如果平台限制或计费变化，以 Fly 文档为准。

## 可选：持久化状态（不是必需）

如果只想让中继记住“哪些群曾在此注册”，可以额外传一个状态文件：

```bash
./asa relay serve 9000 /var/lib/asa/relay-state.json
```

这只是一个便利缓存，**不是权威注册表**。即使删掉它，持有邀请码/群文件的成员仍能在
任意中继重新上线后重新发现彼此。

## 把中继加入群

在有群的机器上：

```bash
asa relay add <服务器地址>:9000
asa relay list
asa invite
```

可以重复执行 `asa relay add` 添加多个中继；客户端会同时连接并注册。之后把新的邀请码
发给成员即可，邀请码里的 `&r=` 会携带所有中继地址。

## 群固定说明

- 群配置保存在每个设备的 `runtime/data/rooms/<群 ID>.json`，群列表在
  `runtime/data/groups/index.json`；不依赖中继保存。
- 群密钥和入口同时保存在群文件中；换设备时导入群文件即可恢复。
- 所有人退出后，任意成员重新上线并向任意一个可用中继注册，其他人即可重新发现。
- 中继全部下线时，群配置和聊天日志仍在本机，重新部署任意中继即可恢复跨网入口。

## 安全建议

- 只开放中继端口，不要暴露其他服务。
- 中继无法解密聊天内容，但能看到群 ID、连接时间和流量大小。
- 建议至少部署两个不同网络/不同提供商的中继，避免单点。
- 正式使用时可用防火墙限制来源，或在中继前加一层 TCP 代理。

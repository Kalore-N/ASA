# 部署 ASA 中继

中继用于让**没有公网 IP** 的设备跨网加入群聊。它只转发已端到端加密的 TCP 字节流，
看不到聊天内容；同时会保存群注册记录，作为群的固定入口。

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

即成功。数据保存在 `deploy/relay-data/relay-state.json`，**不要删除该目录**，
它保存群的注册记录。

验证端口：

```bash
nc -vz <服务器公网IP> 9000
```

## 方案二：Windows / Linux 直接运行

把 `dist/asa`（Linux）或 `dist/asa-windows-x86_64.exe`（Windows）复制到服务器，运行：

```bash
./asa relay serve 9000 /var/lib/asa/relay-state.json
```

Windows：

```powershell
.\asa-windows-x86_64.exe relay serve 9000 D:\asa\relay-state.json
```

需要让防火墙/安全组放行 TCP 9000。

Linux 可用 systemd：

```bash
sudo useradd --system --home /var/lib/asa --shell /usr/sbin/nologin asa
sudo mkdir -p /opt/asa /var/lib/asa
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

## 把中继加入群

在有群的机器上：

```bash
asa relay add <服务器地址>:9000
asa relay list
asa invite
```

之后把新的邀请码发给成员即可；邀请码里的 `&r=` 会携带中继地址。
也可以把长期邀请码（`asa entry list` 或导出群文件）发给对方，里面同样包含中继。

## 群固定说明

- 中继的 `relay-state.json` 保存群 ID 的注册记录；即使所有人退出，记录仍在。
- 当任意持有群密钥的成员重新上线并注册后，其他成员通过中继重新发现并连接。
- 群密钥和入口同时保存在 `room.json` / 导出的群文件中；即使换设备，导入群文件即可恢复群。
- 建议把 `relay-state.json` 放在持久化磁盘/卷上，并定期备份。

## 安全建议

- 只开放中继端口，不要暴露其他服务。
- 中继无法解密聊天内容，但能看到群 ID、连接时间和流量大小。
- 可以部署多个中继，在群里用 `asa relay add` 添加多个地址；客户端当前使用第一个可解析地址。
- 正式使用时建议用防火墙限制来源，或在中继前加一层 TCP 代理。

# ASA · 去中心化 TUI 频道

这是我第一次尝试vibe coding的产物，随便写了点小东西玩玩

当前版本：**1.4.0**

ASA 是一个用 Rust 编写的**完全去中心化**终端频道工具：没有中心服务器，每个参与者都是对等节点。支持局域网自动发现、跨网手动连接、端到端加密、本地聊天记录（JSONL）以及文件传输。

最初创建群的人退出后，频道不会中断——「服主」只是由在场时间决定的逻辑角色，会自动交接给其他节点。

## 特性

- **完全去中心化**：UDP + mDNS 局域网发现，TCP 全网状直连，消息泛洪传播，无中心服务器。
- **服主自动交接**：按有效在场时间选举（创建者锚定群的创建时间），创建者离线后由最早在场的成员接任，负责成员表同步与新成员入口。
- **端到端加密**：群密钥 ChaCha20-Poly1305 加密每条消息，ed25519 签名身份，邀请码之外的节点无法加入。
- **本地聊天记录**：每个节点把消息写入自己的 JSONL 日志，不同步历史（加入前的消息不会补发）。
- **提及与回复**：输入 `@昵称` / `@所有人` 提醒成员；`Ctrl+R` 选择消息或 `/reply <编号>` 精确回复，引用摘要随消息发送。
- **指令补全**：按 `Tab` 补全聊天指令、成员昵称、文件编号和消息编号。
- **跨网中继**：内置 `asa relay`，没有公网 IP 的设备也能通过中继加入群；中继只转发加密字节流。
- **群固定**：群入口、群密钥和群文件可持久化/导出；多入口邀请码与中继让所有人退出后仍能重新恢复群。
- **多群管理**：本机可保存多个群，按自定义名称选择、切换；群配置分别保存在 `rooms/<群 ID>.json`。
- **文件传输**：点对点直连、分块传输、SHA-256 校验，只在双方在线时可用，默认单文件上限 100 MB。
- **跨平台**：使用 `crossterm` + `ratatui`，可在 Linux、macOS、Windows 上运行。

## 快速开始

### 1. 创建群

```bash
asa init --name alice
```

会生成身份、群配置和邀请码。邀请码形如：

```
asa://<群 ID>/<群密钥>@<地址>:<端口>?t=<创建时间>&p=<创建者公钥>
```

### 2. 邀请朋友

把邀请码发给朋友，让对方运行：

```bash
asa join '<邀请码>' --name bob
```

同一局域网内无需其他参数，节点会通过 mDNS 自动发现。跨网时推荐使用内置中继（见下文「跨网与群固定」）；也可以使用公网地址、端口映射或 `/connect <ip:port>`。

### 3. 开始聊天

```bash
asa            # 默认命令，等同于 asa chat
asa invite     # 打印当前群的邀请码
```

直接输入文字回车即发送。输入 `/help` 查看全部命令。

## TUI 启动与频道切换

运行 `asa` 或 `asa chat` 时会先显示不透明的频道选择框，必须选择一个频道后才能进入聊天：

- `↑`/`↓` 选择已保存频道，`Enter` 进入
- `c` 创建新频道
- `j` 粘贴邀请码加入频道
- `r` 修改本机显示的频道名称
- `d` 删除本机保存的频道（不会删除聊天日志和下载文件）
- `q`/`Esc` 退出

进入频道后可用 `/channels` 再次打开选择框，或用 `/switch <名称或 ID>` 直接切换。
切换频道会重启网络节点，但不会删除任何本地文件。

## TUI 操作

| 按键 | 作用 |
| --- | --- |
| `Enter` | 发送消息 |
| `Tab` / `Shift+Tab` | 补全指令、成员、编号；连续按可循环 |
| `Ctrl+R` | 进入回复选择：`↑`/`↓` 选消息，`Enter` 确认，`Esc` 取消 |
| `PgUp` / `PgDn` | 向上/向下翻看聊天记录 |
| `End` | 回到最新消息 |
| `↑` / `↓` | 浏览输入历史（非回复选择模式） |
| `Ctrl+Y` | 把最近内容放入输入行（剪贴板不可用时的兜底） |
| `Ctrl+C` | 退出 |
| `Ctrl+L` | 清屏（不影响本地日志） |
| `Ctrl+U` | 清空输入行 |

输入 `@` 后按 `Tab` 可补全在线成员昵称，支持 `@所有人`。被提及的消息会高亮并在状态栏提示。

### 聊天命令

| 命令 | 作用 |
| --- | --- |
| `/help` | 显示帮助 |
| `/name <昵称>` | 修改昵称并通知频道内 |
| `/rename <名称>` | 修改本机显示的频道名称（仅本地） |
| `/invite` | 显示邀请码 |
| `/peers` | 查看在线成员 |
| `/connect <ip:port>` | 手动连接成员 |
| `/send <成员1,成员2\|@all> <路径>` | 发送文件给多个成员；`/send` 空参进入成员多选 |
| `/copy` | 把最近的邀请码/保存路径复制到系统剪贴板（OSC 52） |
| `/accept <编号>` / `/deny <编号>` | 接受/拒绝收到的文件 |
| `/files` | 查看进行中的传输 |
| `/sync` | 向在线成员请求最近聊天记录（默认也会自动同步） |
| `/channels` | 打开频道选择框 |
| `/switch <名称或 ID>` | 切换到指定频道 |
| `/reply [编号]` | 回复指定消息；不带编号时列出最近消息的编号 |
| `/clear` | 清空当前屏幕 |
| `/quit` | 退出 |

## 命令行选项

```
asa init [--name 昵称] [--port 7777]
asa join <邀请码> [--name 昵称]
asa chat [选项] [--room <群ID>]
asa groups                      列出本地保存的群
asa invite [--port 7777]
asa entry add|remove <地址>     维护群入口
asa entry list                  列出入入口与长期邀请码
asa relay serve [端口] [状态文件]  启动跨网中继（状态文件可选）
asa relay add|remove <地址>     维护中继地址
asa relay list                  列出中继地址
asa export <文件>               导出群文件（含群密钥）
asa import <文件>               导入群文件

通用选项:
  --name <昵称>        指定昵称（覆盖已保存的昵称）
  --port <端口>        TCP 监听端口，默认 7777
  --connect <地址>     手动连接成员，可重复
  --no-mdns            关闭局域网自动发现
  --data-dir <路径>    数据目录
  --max-file-mb <MB>   单文件上限，默认 100，0 表示不限制
  --auto-accept        自动接受所有文件请求（有风险，慎用）
```

未显式指定 `--port` 时，会复用 `runtime/data/settings.json` 中保存的端口。

## 配置文件

首次启动会在 `runtime/data/config.json` 生成全局配置：

```json
{
  "version": 1,
  "download_dir": null,
  "max_file_mb": 100,
  "auto_accept": false,
  "ui": { "theme": "dark", "font_size": 15, "compact": false },
  "network": { "port": null, "mdns": true, "relays": [] },
  "sync": { "enabled": true, "count": 50 }
}
```

- `download_dir`：接收文件的保存目录，支持 `~`；留空使用 `runtime/downloads`。
- `max_file_mb`：单文件上限，0 表示不限制。
- `ui.theme`：`dark` / `light` / `high-contrast`（Android 立即生效；TUI 只保存偏好）。
- `ui.font_size`：Android 字号；TUI 的终端字体由终端模拟器决定，程序无法可靠修改。
- `network.mdns`：局域网自动发现，默认开启，因此局域网开箱即用。
- `network.relays`：跨网中继地址，默认空；部署中继后执行 `asa relay add` 写入。
- `sync.enabled`：加入频道后自动向在线成员拉取最近聊天记录，默认开启。
- `sync.count`：每次请求的最大条数，默认 50，最高 200。同步内容会写入本地 JSONL 日志，并按原时间插入界面；可用 `/sync` 手动再拉取一次。

群配置在 `runtime/data/groups/<频道 ID>.json`，`null` 表示继承全局：

```json
{
  "name": "我的频道",
  "nick": "alice",
  "download_dir": null,
  "max_file_mb": null,
  "auto_accept": null,
  "notify_mentions": null
}
```

- `name`：本机显示的频道名称，只在本地生效，不会广播给其他成员。
- `nick`：当前频道内昵称；聊天页 `/name` 会同时更新它。
- `download_dir` / `max_file_mb` / `auto_accept`：按群覆盖全局。
- `notify_mentions`：是否提示 @ 提及。

## 跨网与群固定

### 1. 部署中继

找一台有公网 IP 的机器（VPS、云主机或朋友常开的服务器），在项目根目录：

```bash
docker compose -f deploy/docker-compose.yml up -d --build
```

或直接运行：

```bash
./asa relay serve 9000
```

详细步骤见 `deploy/README.md`。中继只转发端到端加密后的字节流，看不到聊天内容；默认不保存群状态，不充当中心服务器。可以部署多个中继，客户端会同时连接所有配置的地址。

### 2. 把中继加入群

```bash
asa relay add <公网地址>:9000
asa relay list
asa invite
```

邀请码会带上 `&r=<中继地址>`；普通成员只需粘贴邀请码，不需要再做任何网络配置。可执行多次 `asa relay add` 添加多个中继，任何一个可用即可跨网。

### 3. 多入口与长期邀请码

```bash
asa entry add 203.0.113.10:7777
asa entry list      # 同时显示长期邀请码
```

长期邀请码不含易变的 `@主地址`，只携带 `e=` 入口和 `r=` 中继，适合长期保存。
启动节点时会自动尝试所有入口。

### 4. 群文件（永久入口）

```bash
asa export my-group.json
asa import my-group.json
```

群文件包含群 ID、群密钥、入口和中继地址。把它放到可信位置，即使所有设备丢失，
只要还有这份文件和任意可用入口/中继，就能恢复群。**群文件包含群密钥，请保密。**

### 5. 所有人退出后如何恢复

- 群配置保存在每台设备的 `room.json`，重启 `asa chat` 即可恢复。
- 中继默认无状态，群列表和群密钥始终保存在本地；任意成员重新上线并注册后，其他人可通过任意可用中继重新发现。
- 新设备导入 `my-group.json`，或使用长期邀请码，即可加入。
- 如果中继也下线，群仍保存在本地/群文件中；重新部署中继后即可恢复入口。

## 架构

```
src/
  main.rs       命令行解析与启动
  tui.rs        TUI 布局、渲染与键盘处理
  node.rs       节点主循环：服主选举、消息泛洪、文件传输状态机
  net.rs        TCP 连接、握手、连接任务
  proto.rs      帧格式、Hello、消息信封、加解密
  room.rs       群配置与邀请码
  identity.rs   ed25519 身份与持久化
  crypto.rs     ChaCha20-Poly1305、base64/密钥编码
  discovery.rs  mDNS 服务发现
  relay.rs      跨网中继（服务端 + 客户端）
  history.rs    本地 JSONL 聊天日志
  settings.rs   端口等本地设置
  paths.rs      运行时目录
  util.rs       时间、地址、文件名等工具

deploy/         中继的 Docker / systemd / Fly.io 部署文件
```

### 连接与握手

1. 节点通过 mDNS（服务类型 `_asa._tcp.local.`）或手动地址发现彼此。
2. 建立 TCP 连接后交换 `Hello`：公钥、昵称、通告地址、在场时间、nonce、群密钥证明和签名。
3. 双方互相发送 `Proof`，绑定双方 nonce 与角色，确认对方确实持有群密钥。
4. 同一公钥重复连接时，按「公钥较小的一方发起」的规则保留一条连接。

### 消息传播

聊天消息不依赖服主：发送方把消息泛洪给所有已连接成员，接收方按消息 ID 去重后继续转发。因此服主掉线后，频道仍能继续。

消息载荷除正文外还携带：

- `mentions`：被 @ 的成员公钥。由发送端按当前成员昵称解析，避免昵称含空格或改名后匹配失败；`@所有人` 会展开为全体在线成员。
- `reply`：被回复消息的 id、昵称和摘要。即使接收方没有那条历史（不同步历史），也能显示引用；旧版本节点缺少这些字段时会自动按空值处理。

### 服主选举

每个节点计算候选人的**有效在场时间**：

- 创建者锚定到群的 `created_at`；
- 其他节点不早于 `created_at + 1`；
- 取最小值者当选，平票按公钥字典序。

服主负责广播成员表（PEX）并作为新成员的入口，但聊天本身不依赖服主。

### 文件传输

- 发送方先发送文件请求（文件名、大小、哈希）。
- 接收方 `/accept` 后建立单播传输，数据按 32 KB 分块。
- 传完校验 SHA-256，成功后从 `.part` 重命名到下载目录。
- 需要双方同时在线；默认单文件上限 100 MB。

### 安全与限制

- 消息与文件均使用群密钥加密，身份用 ed25519 签名。
- 邀请码包含群密钥，请只通过可信渠道分享。
- **历史同步范围有限**：默认向在线成员拉取最近 50 条，不能补全完整历史或离线成员的内容。
- 跨网推荐使用内置中继；中继只转发加密后的字节流，但仍能看到群 ID、连接时间和流量大小。
- 中继默认无状态，不保存群成员表；群固定依赖本地群文件/邀请码，任何可用中继都能参与发现。
- 可配置多个中继，客户端会同时连接；单个中继故障时其他中继仍可用。
- 没有离线消息队列，对方离线时发送的文件请求会失败。
- 文件传输不限制接收方磁盘空间，请自行注意。

## 平台与安装包

GitHub Releases 提供各平台的可运行产物；源码仓库不包含构建产物，`dist/` 在本地生成。

| 平台 | 文件 | 说明 |
| --- | --- | --- |
| Linux x86-64 | `asa-1.4.0-linux-x86_64` | musl 静态单文件，无 glibc 依赖 |
| Windows x86-64 | `asa-1.4.0-windows-x86_64.exe` | MSVC 构建，控制台程序 |
| Android arm64 / x86_64 | `asa-1.4.0-android.apk` | Kotlin + Compose 图形界面，可直接安装 |
| Arch Linux x86-64 | `asa-1.4.0-1-x86_64.pkg.tar.zst` | 含二进制、man、桌面入口与图标 |
| Debian / Ubuntu amd64 | `asa_1.4.0_amd64.deb` | 含二进制、man、桌面入口与图标 |
| Fedora / RHEL x86-64 | `asa-1.4.0-1.x86_64.rpm` | 含二进制、man、桌面入口与图标 |

### Linux 发行版包

```bash
# Arch
sudo pacman -U asa-1.4.0-1-x86_64.pkg.tar.zst

# Debian / Ubuntu
sudo apt install ./asa_1.4.0_amd64.deb

# Fedora / RHEL
sudo dnf install ./asa-1.4.0-1.x86_64.rpm
```

安装后直接运行 `asa`，或从应用菜单打开 ASA。安装包是静态二进制，不依赖系统 Rust 工具链。

### Windows

在 Windows Terminal、PowerShell 或 CMD 中运行：

```powershell
.\asa-windows-x86_64.exe --help
.\asa-windows-x86_64.exe init --name alice
.\asa-windows-x86_64.exe
```

### Android

1. 把 `asa-android.apk` 传到手机并允许安装未知来源应用。
2. 打开 ASA，填写昵称，选择「创建群」或「加入群」。
3. 加入群时粘贴邀请码。如果邀请码里包含中继（`&r=`），会自动走中继跨网，不需要填写地址；否则可填电脑的局域网地址，例如 `192.168.1.8:7777`。
4. 创建群后，点右上角邀请码图标，复制邀请码发给朋友。

Android 版说明：

- 页面采用小圆角：消息气泡 8dp，按钮、输入框和卡片 6dp。
- 设置页包含下载保存位置与复制、单文件上限、主题、字号、紧凑布局、mDNS、中继、自动接受；与 `config.json` 字段对应。
- 启动后进入「选择频道」页面，按本机名称选择保存的频道；聊天菜单可「切换频道」。
- 「加入频道」不会覆盖已有频道，新频道会追加到列表；「创建频道」每次创建新频道。
- 新消息在应用后台时发送系统通知；应用在前台时不打扰。首次启动会请求通知权限。
- 使用前台服务保持后台连接，常驻通知显示「ASA 正在保持连接」。
- 返回键从频道回到频道列表；在列表再按返回才回到桌面。只有菜单里的「退出频道」并确认后才会停止连接。
- 群名称可在聊天菜单中修改，仅本机显示。
- 支持中继跨网；邀请码里的 `&r=` 会自动生效。
- 发送文件支持多选成员和全选。
- mDNS 默认关闭，没有中继时需要填写可访问的 `ip:port`。
- 文件发送使用系统文件选择器；接收的文件保存在应用私有目录，可在传输完成提示中查看。
- 系统在文件选择器期间可能冻结后台进程，应用会在返回后等待约 2 秒恢复连接再发送文件。
- 需要 Android 8.0（API 26）及以上。

## 运行时数据

默认数据目录为 `~/Documents/ASA`（可用 `--data-dir` 修改）：

```
runtime/
  data/        身份、全局配置、群配置
  data/rooms/  每个群的配置（可多个）
  data/groups/ 群名称、昵称、列表索引
  logs/        聊天记录（JSONL）
  downloads/   接收到的文件
  tmp/         传输中的临时文件
```

Android 的数据目录位于应用私有空间：`/data/user/0/com.asa.chat/files/asa/runtime/`。

## 构建

### Linux / macOS

需要 Rust stable 工具链。

```bash
cargo build --release
./target/release/asa --help
```

### Windows（交叉编译）

需要 `cargo-xwin` 与 MSVC target：

```bash
rustup target add x86_64-pc-windows-msvc
cargo install cargo-xwin
cargo xwin build --release --target x86_64-pc-windows-msvc
```

### Linux 发行版包

需要 `makepkg`、`fakeroot`、`ar`、`gzip`，以及 `rpmbuild`（RPM 部分）：

```bash
bash packaging/build-packages.sh
```

输出在 `dist/packages/`。`packaging/arch/PKGBUILD`、`packaging/rpm/asa.spec` 和脚本都可以直接复用。

### Android

需要 Android SDK、NDK、JDK 21 与 Gradle。核心库通过 `cargo-ndk` 编译，界面在 `android/`：

```bash
cargo ndk -t arm64-v8a -P 26 -o android/app/src/main/jniLibs build --release --lib
cargo ndk -t x86_64 -P 26 -o android/app/src/main/jniLibs build --release --lib
cd android && ./gradlew :app:assembleDebug
```

Android 工程使用 `android/local.properties` 指向本机 SDK；`android/keystore/` 中是测试签名，正式发布前请替换。

```bash
echo "sdk.dir=/path/to/Android/Sdk" > android/local.properties
```

## 测试

```bash
cargo test
```

包含单元测试与端到端测试（聊天、@/回复、文件传输、服主交接、错误群密钥无法加入）。

## 许可证

MIT

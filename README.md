# ASA · 去中心化 TUI 频道

这是我第一次尝试vibe coding的产物，随便写了点小东西玩玩

当前版本：**1.7.3**

ASA 是一个用 Rust 编写的**完全去中心化**终端频道工具：没有中心服务器，每个参与者都是对等节点。支持局域网自动发现、跨网手动连接、端到端加密、本地聊天记录（JSONL）以及文件传输。

最初创建群的人退出后，频道不会中断——「服主」只是由在场时间决定的逻辑角色，会自动交接给其他节点。

## 特性

- **完全去中心化**：UDP + mDNS 局域网发现，TCP 全网状直连，消息泛洪传播，无中心服务器。
- **服主自动交接**：按有效在场时间选举（创建者锚定群的创建时间），创建者离线后由最早在场的成员接任，负责成员表同步与新成员入口。
- **端到端加密**：群密钥 ChaCha20-Poly1305 加密每条消息，ed25519 签名身份，邀请码之外的节点无法加入。
- **本地聊天记录**：每个节点把消息写入自己的 JSONL 日志，不同步历史（加入前的消息不会补发）。
- **提及与回复**：输入 `@昵称` / `@所有人` 提醒成员；`Ctrl+R` 选择消息或 `/reply <编号>` 精确回复，引用摘要随消息发送。
- **指令补全**：按 `Tab` 补全聊天指令、二级参数（`/et`、`/call`、`/voice`、`/switch`）、成员昵称、文件路径、文件编号和消息编号；补全弹窗会高亮当前选中项。
- **跨网中继**：内置 `asa relay`，没有公网 IP 的设备也能通过中继加入群；中继只转发加密字节流。
- **群固定**：群入口、群密钥和群文件可持久化/导出；多入口邀请码与中继让所有人退出后仍能重新恢复群。
- **多群管理**：本机可保存多个群，按自定义名称选择、切换；群配置分别保存在 `rooms/<群 ID>.json`。
- **文件传输**：点对点直连、分块传输、SHA-256 校验，只在双方在线时可用，默认单文件上限 100 MB。
- **语音**：1:1 通话、语音消息，以及频道常驻多人语音房；Android 支持一键录音和可点击语音条。
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
- `d` 删除本机保存的频道：同时清理本机频道设置（名称/EasyTier/加入密码），
  但不删除聊天日志和下载文件
- `q`/`Esc` 退出

进入频道后可用 `/channels` 再次打开选择框，或用 `/switch <名称或 ID>` 直接切换。
切换频道会重启网络节点，但不会删除任何本地文件。

### TUI 浏览模式（Vim）

TUI 默认进入**浏览模式**，不会直接接收文字：

- `i` / `a` / `Enter`：进入输入模式
- 输入模式下 `Enter` 发送，`Esc` 返回浏览模式
- 浏览模式下 `j` / `k` 向下/向上滚动，`g` / `G` 到最早/最新
- `PageUp` / `PageDown`、鼠标滚轮也可以滚动
- 聊天区右侧显示滚动条，底部显示“已向上 N 行”
- `/mouse off` 关闭鼠标捕获后可直接用鼠标拖选文本；`/mouse on` 恢复滚轮
- 鼠标捕获开启时，多数终端也可以按住 `Shift` 拖选文本
- `Ctrl+C` 退出，`Ctrl+L` 清屏，`Ctrl+U` 清空输入

可以在配置文件中修改：

```json
"ui": {
  "vim_mode": true,
  "scroll_step": 3,
  "history_lines": 200
}
```

- `vim_mode`：`true` 常驻 Vim 风格；`false` 恢复传统“直接输入”模式
- `scroll_step`：`j/k` 和鼠标滚轮每次滚动行数
- `history_lines`：启动时从本地 JSONL 加载并显示的历史条数，0 表示不加载

## EasyTier 内网穿透（可选）

ASA 可以把 [EasyTier](https://github.com/EasyTier/EasyTier) 当作可选的虚拟局域网层：
两台设备加入同一个 EasyTier 网络后，ASA 的 TCP 流量可以直接通过虚拟 IP 互通，
不再依赖应用层中继。

EasyTier 是**可选**的。没有节点、没有权限或启动失败时，ASA 会自动回退到
局域网 mDNS / 多中继，不阻塞聊天。

### 添加并选择节点

节点地址来自 EasyTier 公共节点列表（例如 `https://info.qtet.cn/uptime/easytier`），
格式为 `tcp://host:11010`、`udp://host:11010`、`ws://host` 或 `wss://host`。
部分公共节点地址被打码（含 `*`），需要去 EasyTier QQ 群获取完整地址。

- TUI 频道大厅按 `n` 添加；聊天内 `/et add <地址> [更多地址…]`、`/et list`、`/et remove <编号>`
- 可以保存多个节点，但**每个频道只使用其中一个**：`/et use <编号>` 选择；
  不选择时使用列表中第一个
- `/et test [编号]` 用 TCP 连接测试节点，显示“通/不通”和毫秒延迟；
  建议先测试再选择，避免把不通的节点设为当前节点
- Android：设置 → EasyTier 节点 → 添加；也可以点「从 qtet 监控页一键导入」。
  每个节点右侧有「测试」按钮（显示延迟），左侧单选按钮选择当前频道使用的节点；
  选择按频道保存，重新进入频道后生效
- 没有节点时，打开 ASA 会在频道大厅提示添加，但允许跳过

节点列表和选择都保存在本机；删除节点会同时清理所有频道里残留的旧节点和选择，
避免重启后旧节点又出现。

### 为频道开启

- TUI：频道大厅按 `e` 切换当前频道的 EasyTier；聊天内 `/et on` / `/et off`
- Android：频道列表右侧开关；当前频道切换后立即启动/停止 ET 和 VPN，非当前频道只保存偏好
- 默认关闭，按频道单独选择；进入已开启的频道会自动启动

### 邀请码

邀请码已经包含频道密钥，所以 EasyTier 的网络名和密钥由频道密钥派生，
不需要额外传递。邀请码会携带本机保存的 EasyTier 节点池（`&et=`，可多个）：
受邀者加入时会把它们并入自己的节点列表并默认开启 ET，
然后可以使用 `/et use` 或 Android 设置里的单选按钮选择实际节点。

```
&et=<base64url(节点地址)>
```

开启 EasyTier 后，邀请码的主地址会变成宿主的 EasyTier 虚拟 IP
（例如 `asa://...@10.144.23.74:7777?...&et=dGNwOi8v...`），
同时保留 `&e=` 局域网备用入口。加入方解析 `&et=` 后会自动为该频道开启 EasyTier。

每个成员用 ASA 公钥派生固定的 `10.144.x.y/16` 虚拟 IP；宿主实际使用的地址
会写进邀请码，不依赖 DHCP。

### 权限

- **Linux**：EasyTier 需要 `CAP_NET_ADMIN`。安装包会在安装时执行
  `setcap cap_net_admin,cap_net_raw+ep /usr/lib/asa/easytier-core`；
  便携版首次使用执行：
  `sudo setcap cap_net_admin,cap_net_raw+ep ./easytier-core`
- 如果 `/et status` 显示 `Operation not permitted`，就是缺少上面的 TUN 权限；
  此时节点测试可能显示“通”，但 EasyTier 仍然无法建立虚拟网络
- **Windows**：需要以管理员身份运行 `asa.exe`，EasyTier 才能创建 TUN 网卡
- **Android**：使用 VpnService，不需要 root；首次开启会弹出系统 VPN 授权

### 与其他 VPN 共存

Android 同一用户同时只能有一个 **VpnService**。ASA 使用 per-app VPN：
只把 ASA 自己的流量路由进 EasyTier TUN，其他应用不受影响。
但如果设备上已经有另一个 VpnService（大多数 VPN/代理应用），
启动 ASA 的 EasyTier 会替换它。需要真正同时运行两个 VPN 时，
可以把 ASA 装进工作资料（Shelter / Island / Android for Work），
个人资料和工作资料可以各自保持一个 VPN。

如果希望完全不影响现有 VPN，可以在频道列表中关闭该频道的 EasyTier，
继续使用 ASA 自带的局域网/中继。

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
| `v` | 加入/退出频道语音房（浏览模式） |

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
| `/mouse [on\|off]` | 切换鼠标捕获；关闭后可直接拖选文本 |
| `/accept <编号>` / `/deny <编号>` | 接受/拒绝收到的文件 |
| `/files` | 查看进行中的传输 |
| `/sync` | 按 `sync.count` 条向在线成员请求历史记录；0 表示全部，默认 50 |
| `/channels` | 打开频道选择框 |
| `/switch <名称或 ID>` | 切换到指定频道 |
| `/et [status\|on\|off\|add <地址>\|list\|remove <编号>\|use <编号>\|test [编号]]` | 管理 EasyTier 节点、选择本频道节点、测试延迟 |
| `/call <昵称> \| accept \| reject \| hangup \| mute \| ptt` | 1:1 语音通话 |
| `/voice join` / `leave` / `mute` / `members` | 加入、退出、静音频道语音房、查看成员 |
| `/voice <昵称> \| stop \| play <路径>` | 录制、发送、播放语音消息 |
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
  --data-dir <路径>    数据目录；默认 ~/.config/asa，指定后使用 <路径>/runtime
  --max-file-mb <MB>   单文件上限，默认 100，0 表示不限制
  --auto-accept        自动接受所有文件请求（有风险，慎用）
```

未显式指定 `--port` 时，会复用 `~/.config/asa/settings.json` 中保存的端口。

## 配置文件

首次启动会在 `~/.config/asa/config.json` 生成全局配置：

```json
{
  "version": 1,
  "download_dir": null,
  "max_file_mb": 100,
  "auto_accept": false,
  "ui": {
    "theme": "dark",
    "font_size": 15,
    "compact": false,
    "history_lines": 200,
    "vim_mode": true,
    "scroll_step": 3
  },
  "network": { "port": null, "mdns": true, "relays": [] },
  "sync": { "enabled": true, "count": 50 },
  "easytier": { "nodes": [], "auto_enable": true },
  "voice": {
    "enabled": true,
    "codec": "pcm",
    "sample_rate": 16000,
    "channels": 1,
    "frame_ms": 20,
    "push_to_talk": false,
    "record_dir": null,
    "input_device": null,
    "output_device": null
  }
}
```

- `download_dir`：接收文件的保存目录，支持 `~`；留空使用 `~/Downloads`（遵循 `XDG_DOWNLOAD_DIR`）。
- `max_file_mb`：单文件上限，0 表示不限制。
- `ui.theme`：`dark` / `light` / `high-contrast`（Android 立即生效；TUI 只保存偏好）。
- `ui.font_size`：Android 字号；TUI 的终端字体由终端模拟器决定，程序无法可靠修改。
- `ui.history_lines`：TUI/Android 启动时加载的本地历史条数，默认 200。
- `ui.vim_mode`：TUI 是否常驻 Vim 浏览模式，默认开启。
- `ui.scroll_step`：TUI 单次滚动行数，默认 3。
- `network.mdns`：局域网自动发现，默认开启，因此局域网开箱即用。
- `network.relays`：跨网中继地址，默认空；部署中继后执行 `asa relay add` 写入。
- `sync.enabled`：加入频道后自动向在线成员拉取最近聊天记录，默认开启。
- `sync.count`：每次请求的条数，默认 50；设为 0 表示全部（最多 5000 条）。同步会从**本地全部 JSONL 历史**中取记录，并写入本地日志、按原时间插入界面；可用 `/sync` 手动触发。
- `easytier.nodes`：本机保存的 EasyTier 节点池，默认空；每个频道用群配置里的 `et_node` 选择其中一个，为空时用池中第一个。
- `easytier.auto_enable`：有新节点时，新频道默认开启 EasyTier，默认 true。
- `voice.enabled`：启用语音通话和语音消息。
- `voice.codec` / `sample_rate` / `channels` / `frame_ms`：目前固定 PCM 16kHz 单声道，20ms/帧。
- `voice.push_to_talk`：TUI 使用按住说话；false 为全双工。
- `voice.record_dir`：语音消息保存目录；留空使用下载目录下的 `voice/`（默认 `~/Downloads/voice`）。
- `voice.input_device` / `output_device`：Linux ALSA 设备或 Windows ffmpeg dshow 设备名。

群配置在 `~/.config/asa/groups/<频道 ID>.json`，`null` 表示继承全局：

```json
{
  "name": "我的频道",
  "nick": "alice",
  "download_dir": null,
  "max_file_mb": null,
  "auto_accept": null,
  "notify_mentions": null,
  "easytier_enabled": null,
  "et_node": null
}
```

- `name`：本机显示的频道名称，只在本地生效，不会广播给其他成员。
- `nick`：当前频道内昵称；聊天页 `/name` 会同时更新它。
- `download_dir` / `max_file_mb` / `auto_accept`：按群覆盖全局。
- `notify_mentions`：是否提示 @ 提及。
- `easytier_enabled`：是否为此频道开启 EasyTier 内网穿透；`null` 表示关闭。
- `et_node`：本频道选中的 EasyTier 节点；`null` 或节点已被删除时使用节点池第一个。

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

## 语音通话与语音消息

### 1:1 实时通话

- 通话邀请/接听/拒绝/挂断走现有加密 TCP
- 音频使用 PCM 16kHz 单声道、20ms/帧
- 优先走 UDP；2 秒内收不到 UDP 自动回退到现有加密 TCP
- 通话不录音，只有你手动录制的语音消息会保存

TUI：

- `/call <昵称>` 发起，来电时 `/call accept` / `/call reject`
- 通话中 `/call mute`、`/call hangup`、`/call ptt`
- 浏览模式下 `m` 静音、`h` 挂断；`voice.push_to_talk=true` 时 `p` 切换发送

Android：

- 聊天页顶部电话按钮选择成员发起通话
- 来电弹窗接听/拒绝；通话条支持静音、免提、挂断
- 麦克风权限和前台服务 `microphone`

### 频道语音房（多人同时通话）

- 进入频道后 `/voice join`（或浏览模式按 `v`）加入，`/voice leave` 退出；不加入不会收到音频
- 房内所有成员互相收发音频，适合小规模常驻语音（建议不超过 8 人）
- `/voice mute` 静音自己的麦克风，`/voice members` 查看房内成员
- 音频帧使用现有加密 TCP 连接广播（每个 20ms 帧单独签名/加密），不额外开放端口
- 语音房与 1:1 通话互斥：在语音房中发起/接听通话会被拒绝，避免同时占用麦克风
- Android 顶部「语音房」按钮可加入/退出、查看成员、静音和免提

### 语音消息

- TUI `/voice <昵称>` 开始录音，`/voice stop` 停止并发送；`/voice play <路径>` 播放
- Android 输入栏麦克风按钮：点一下开始录音，再点停止；未选接收人时会弹出成员选择
- Android 聊天流里收到的语音消息显示为可点击语音条，点击播放，再次点击停止
- 语音消息保存为 WAV，走现有加密文件传输；对方接受后保存到下载目录
- 通话不自动录音，避免误录双方声音

### 没有声音时

- TUI 会在音频工具启动失败或中途退出时直接提示原因（例如设备被占用、采样率不支持），并自动挂断
- 默认按顺序尝试 `default` → `pulse` → `pipewire` → 各声卡的 `plughw:card,device`（只用能跑起来的那个），
  并在状态栏/聊天区显示实际使用的设备；`arecord -l` 探测有 1.5 秒超时，不会卡住界面
- 仍无声时在 `config.json` 的 `voice.input_device` / `voice.output_device` 手写设备名（此时不再自动回退），
  例如 `default`、`pulse` 或 `plughw:1,2`；`arecord -l`、`aplay -l` 可列出设备
- 如果对方能听到你但你听不到对方，检查 `voice.output_device`；反之检查 `voice.input_device`
- 两个程序同时占用同一块声卡时，后启动的一方可能失败；关闭占用麦克风的程序或配置不同的设备

### 依赖

- Linux TUI：`arecord` / `aplay`（alsa-utils）、`ffplay`（ffmpeg）
- Windows TUI：`ffmpeg` / `ffplay`，并在 `voice.input_device` 填写 dshow 设备名
- Android：使用系统 AudioRecord/AudioTrack/MediaPlayer，无需额外安装

## 局域网频道发现

宿主进入频道后，会在局域网广播频道目录（mDNS，`_asa-channel._tcp.local.`）。
客户端无需先拿到邀请码，就可以发现并加入：

- **创建频道时** TUI 会询问是否设置“频道加入密码”；直接回车表示不设密码
- 已有频道可在 TUI 大厅按 `p` 设置/清除加入密码
- TUI 大厅按 `f` 打开“局域网频道”列表，选择后输入密码加入
- Android 在「加入频道」页自动搜索并显示“局域网频道”，点选后输入密码加入
- 首次加入成功后保存邀请码，并保存**用频道密钥加密的加入密码**
- 再次启动时，如果发现已保存的频道，大厅会自动选中它；按 Enter 自动认证

密码验证使用 PBKDF2-SHA256（随机 salt，约 21 万次迭代）和 HMAC 挑战应答；
邀请码通过密码派生密钥加密后传输，不在局域网明文发送密码。

- 没有设置密码的频道会以“开放”形式广播，局域网内任何人都能直接加入
- 不想暴露频道时，请设置加入密码；或不要长期停留在频道内
- mDNS 只用于局域网发现；跨网仍走邀请码、中继或 EasyTier
- Android 模拟器之间可能无法互相发现 mDNS，请用两台真机在同一 Wi-Fi 测试

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
- EasyTier 节点地址可能被打码；打码地址无法使用，需要从 EasyTier QQ 群获取完整地址。
- Linux 便携版需要手动 `setcap`，Windows 需要管理员权限；否则 EasyTier 会回退到局域网/中继。
- Android 同一用户只能有一个 VpnService；ASA 使用 per-app VPN，只影响 ASA 自身，但会替换其他 VpnService。
- `/sync` 会把本地 JSONL 中的历史消息发给在线成员；如果不想共享，请关闭 `sync.enabled`，或不要手动触发同步。
- 局域网“开放”频道没有密码，同网段任何人都能加入；敏感频道请设置加入密码。
- mDNS 不认证频道目录，局域网攻击者可以看到频道名；密码只保护加入过程。

## 平台与安装包

GitHub Releases 提供各平台的可运行产物；源码仓库不包含构建产物，`dist/` 在本地生成。

| 平台 | 文件 | 说明 |
| --- | --- | --- |
| Linux x86-64 | `asa-1.7.3-linux-x86_64.tar.gz` | 含 `asa`、`easytier-core`、`easytier-cli` |
| Windows x86-64 | `asa-1.7.3-windows-x86_64.zip` | 含 `asa.exe`、EasyTier、`wintun.dll` |
| Android arm64 | `asa-1.7.3-android-arm64.apk` | 真机推荐；含 arm64 EasyTier 原生库 |
| Android x86_64 | `asa-1.7.3-android-x86_64.apk` | 模拟器 / x86_64 平板 |
| Arch Linux x86-64 | `asa-1.7.3-1-x86_64.pkg.tar.zst` | 含二进制、EasyTier、man、桌面入口与图标 |
| Debian / Ubuntu amd64 | `asa_1.7.3_amd64.deb` | 含二进制、EasyTier、man、桌面入口与图标 |
| Fedora / RHEL x86-64 | `asa-1.7.3-1.x86_64.rpm` | 含二进制、EasyTier、man、桌面入口与图标 |

### Linux 发行版包

```bash
# Arch
sudo pacman -U asa-1.7.3-1-x86_64.pkg.tar.zst

# Debian / Ubuntu
sudo apt install ./asa_1.7.3_amd64.deb

# Fedora / RHEL
sudo dnf install ./asa-1.7.3-1.x86_64.rpm
```

安装后直接运行 `asa`，或从应用菜单打开 ASA。安装包是静态二进制，不依赖系统 Rust 工具链。

### Windows

在 Windows Terminal、PowerShell 或 CMD 中运行：

```powershell
.\asa.exe --help
.\asa.exe init --name alice
.\asa.exe
```

### Android

1. 把 `asa-android.apk` 传到手机并允许安装未知来源应用。
2. 打开 ASA，填写昵称，选择「创建群」或「加入群」。
3. 加入群时粘贴邀请码。如果邀请码里包含中继（`&r=`），会自动走中继跨网，不需要填写地址；否则可填电脑的局域网地址，例如 `192.168.1.8:7777`。
4. 创建群后，点右上角邀请码图标，复制邀请码发给朋友。

Android 版说明：

- 聊天页“更多”菜单提供：同步聊天记录、进行中的传输、打开下载目录
- 进入频道时会从本地 JSONL 加载最近历史；`/sync` 可以继续向在线成员拉取
- 频道列表顶部会显示局域网内发现的频道，设置密码的频道需要输入密码
- 查看文件传输进度，或直接打开系统文件管理器中的 ASA 下载目录
- 设置页「关于」显示应用版本、核心版本；点「检查更新」连接 GitHub Releases 检测新版本
  - 只在手动点击时联网，不会后台轮询
  - 发现新版可选择「下载并安装」：应用内下载 APK（带进度条），完成后调起系统安装器
  - 首次安装需允许「安装未知应用」；下载文件保存在应用私有目录
- 频道列表右侧垃圾桶可删除频道，确认框可选「同时删除该频道的聊天记录」
  - 删除只影响本机，不会解散其他成员的频道
  - 删除正在连接的频道会先自动退出（停止节点与前台服务）
- 聊天页电话按钮发起 1:1 语音通话；顶部语音房按钮加入频道多人语音
- 输入栏麦克风按钮录制语音消息；聊天流语音条点击播放/停止
- 首次使用语音会请求麦克风权限
- 长按消息正文进入系统文本选择，可只复制正文；向左滑动消息可回复

- 页面采用小圆角：消息气泡 8dp，按钮、输入框和卡片 6dp。
- 设置页包含下载保存位置与复制、单文件上限、主题、字号、紧凑布局、mDNS、中继、自动接受、版本号与检查更新；与 `config.json` 字段对应。
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

桌面端默认使用 XDG 标准位置：

```
~/.config/asa/            身份、全局配置、群配置
  config.json             全局配置
  identity.json           ed25519 身份
  rooms/<群ID>.json        每个频道的配置（可多个）
  groups/<群ID>.json       频道名称/昵称/EasyTier 选择
  groups/index.json        频道列表索引
~/.local/state/asa/logs/  聊天记录与 EasyTier 日志
~/Downloads/              接收到的文件（遵循 XDG_DOWNLOAD_DIR）
~/.cache/asa/tmp/         传输中的临时文件
```

- `--data-dir <路径>` 会改用自包含布局 `<路径>/runtime/{data,logs,downloads,tmp}`，
  适合便携包、测试或把数据放在指定磁盘。
- 从旧版本升级时，首次启动会自动把 `~/Documents/ASA/runtime` 下的身份、频道和日志
  复制到新位置；旧下载文件保留在原处。
- Android 的数据目录仍位于应用私有空间：`/data/user/0/com.asa.chat/files/asa/runtime/`。

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

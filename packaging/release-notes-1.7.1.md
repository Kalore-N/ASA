ASA v1.7.1：语音修复、补全增强与频道语音房

### 修复
- TUI 1:1 语音通话「接通但没声音 / 对方听不到」
  - 不再启动时同步执行 `arecord -l` 猜设备（可能阻塞整个 TUI）
  - 不再只绑定一个设备：按 `default` → `pulse` → `pipewire` → 各声卡 `plughw:card,device`
    依次探测，使用第一个能真正运行的设备
  - `arecord -l` / `aplay -l` 探测带 1.5 秒超时，不会阻塞 TUI；实际使用的设备会显示出来
  - 配置了 `voice.input_device` / `voice.output_device` 时只使用指定设备，便于手动排查
  - 录音/播放工具启动后立即退出会直接报错并自动挂断，提示真实的 arecord/aplay 错误
  - 通话中途录音或播放进程退出也会提示并终止通话，不再静默无声
    - 修复语音帧高速产生时节点事件队列被饿死的问题：去掉 `tokio::select!` 的 `biased`，
    并在 TUI 侧按帧间隔限速发送
- TUI 补全弹窗高亮同时应用到“文件接收人多选”弹窗
- TUI 二级补全缺失
  - 新增 `/et`、`/call`、`/voice`、`/switch` 的子命令/参数补全
  - `/voice play` 支持文件路径补全，`/et remove` 支持节点编号补全
  - `/call`、`/voice` 仍会混合补全在线成员昵称
- TUI 补全弹窗无高亮
  - 选中项使用黄底黑字加粗高亮，并显示 `▸` 指示

### 新增
- 频道常驻语音房（多人同时通话，类似 Discord）
  - TUI：`/voice join`、`/voice leave`、`/voice mute`、`/voice members`；浏览模式按 `v` 切换
  - Android：顶部「语音房」按钮，可加入/退出、查看成员、静音、免提
  - 音频帧通过现有加密 TCP 连接广播，所有房内成员互相收发，不额外开放端口
  - 与 1:1 通话互斥，避免同时占用麦克风；成员离线自动移出语音房
- Android 语音输入按钮
  - 输入栏新增麦克风按钮，点一下开始录音，再点停止
  - 未选择接收人时，停止后自动弹出成员选择
- Android 语音条播放
  - 收到的 `.wav` 语音消息在聊天流中显示为可点击语音条
  - 点击播放，再次点击停止；自己发送的语音也会显示语音条
- 服务端/协议新增 `voice_room_join` / `voice_room_leave` / `voice_room_state` /
  `voice_room_frame` 载荷，旧版本节点会忽略这些消息

### 验证
- Rust：74 unit + 9 e2e（新增语音房加入/退出与音频帧转发测试）通过
- TUI 双实例（假音频设备）验证：
  - 1:1 通话双向接通并收发 PCM 帧
  - 语音房两人加入/退出、成员数同步、互斥规则生效
  - 录音设备失败时提示真实错误并自动挂断
- APK：`arm64-v8a` / `x86_64` release 构建通过，arm64 已安装到真机启动

### 兼容性
- 版本号 1.7.1，Android `versionCode 21`
- 配置格式不变；旧邀请码、旧频道可直接使用
- 语音房需要双方都升级到 1.7.1

### 资产
- Linux：`asa-1.7.1-linux-x86_64.tar.gz`
- Windows：`asa-1.7.1-windows-x86_64.zip`
- Android arm64：`asa-1.7.1-android-arm64.apk`
- Android x86_64：`asa-1.7.1-android-x86_64.apk`
- Arch：`asa-1.7.1-1-x86_64.pkg.tar.zst`
- Debian/Ubuntu：`asa_1.7.1_amd64.deb`
- Fedora/RHEL：`asa-1.7.1-1.x86_64.rpm`

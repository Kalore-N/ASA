ASA v1.7.0：语音通话、语音消息与配置补全

### 新增
- 1:1 实时语音通话（TUI / Android）
  - PCM 16kHz 单声道、20ms/帧
  - 通话邀请、接听、拒绝、挂断走现有加密 TCP
  - 音频优先走 UDP，2 秒内收不到 UDP 自动回退 TCP
  - TUI 支持全双工和按住说话两种模式，`m` 静音、`h` 挂断
  - Android 通话条支持静音、免提、挂断，使用麦克风前台服务
- 语音消息（TUI / Android）
  - 手动开始/结束录音，保存为 WAV
  - 复用现有加密文件传输发送，不录制通话内容
  - Android 收到 WAV 后可直接“播放最近语音”
- `voice` 配置节：`enabled`、`codec`、`sample_rate`、`channels`、`frame_ms`、`push_to_talk`、`record_dir`、`input_device`、`output_device`
- EasyTier 快捷与自动使用
  - 新增 `easytier.auto_enable`，默认开启
  - 全局存在节点时，新频道默认开启 EasyTier，进入频道自动启动
  - 保留原有 `/et add|list|remove|on|off` 快捷管理

### 修复
- 配置升级到 schema v2：启动时递归补齐缺失字段，同时保留未知字段
- 修复 Android 只添加 EasyTier 节点时可能生成精简配置、丢失文件保存位置/大小限制等字段的问题
- Android 设置保存不再丢失 `voice`、`easytier`、`sync` 等配置
- 录音设备自动探测：Linux 下未配置 `voice.input_device` 时，自动选择第一个 `plughw:card,device`
- Android 语音消息保存目录支持设置中的“语音保存目录”，默认使用下载目录

### 文档与打包
- README、man page 增加语音、配置字段和依赖说明
- Arch：依赖 `libcap`、`ffmpeg`、`alsa-utils`
- Debian/Ubuntu：依赖 `libcap2-bin`、`ffmpeg`、`alsa-utils`
- Fedora/RHEL：依赖 `libcap`、`alsa-utils`，推荐 `ffmpeg`

### 兼容性
- 旧配置文件和旧邀请码仍可用，启动时自动补全
- 旧版本客户端不会处理语音帧；语音功能需要双方都升级到 1.7.0
- 语音消息仍是普通加密文件，对方需要接受后才会保存

### 依赖
- Linux TUI：`arecord` / `aplay`（alsa-utils）、`ffplay`（ffmpeg）
- Windows TUI：`ffmpeg` / `ffplay`，并在 `voice.input_device` 填写 dshow 设备名
- Android：使用系统 AudioRecord / AudioTrack / MediaPlayer，无需额外安装

### 已知限制
- Windows TUI 语音需要手动填写 ffmpeg dshow 设备名
- Android 模拟器的音频硬件能力有限，建议用真机验证通话音质
- EasyTier 在部分网络/设备上仍可能出现“已启动但连不上对端”，建议检查节点可达性、NAT 和邀请码地址

### 资产
- Linux：`asa-1.7.0-linux-x86_64.tar.gz`
- Windows：`asa-1.7.0-windows-x86_64.zip`
- Android arm64：`asa-1.7.0-android-arm64.apk`
- Android x86_64：`asa-1.7.0-android-x86_64.apk`
- Arch：`asa-1.7.0-1-x86_64.pkg.tar.zst`
- Debian/Ubuntu：`asa_1.7.0_amd64.deb`
- Fedora/RHEL：`asa-1.7.0-1.x86_64.rpm`

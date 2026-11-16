ASA v1.7.4：EasyTier 多节点选择、延迟测试与标准配置目录

### 新增
- EasyTier 节点池支持保存多个公共节点
  - 每个频道单独选择实际使用的节点：TUI `/et use <编号>`，Android 设置中单选
  - 未选择或节点被删除时自动回退到节点池第一个
- 节点测试
  - TUI `/et test [编号]`，Android 每个节点「测试」/「测试全部」
  - 通过 TCP 连接显示“通/不通”和毫秒延迟
- 邀请码携带完整 EasyTier 节点池（多个 `&et=`）
  - 加入方自动并入自己的节点池，并选择第一个可用节点
- EasyTier TUN 权限诊断
  - 启动失败时在聊天区显示真实原因和 `setcap` 命令，而不是静默回退

### 修复
- 删除 EasyTier 节点后，`/et on`、`/et list` 仍显示旧节点
  - 频道重启时重新读取配置，不再复用启动时的旧节点列表
  - 以全局节点池为唯一来源，旧频道节点自动迁移
  - 删除时清理所有频道里残留的 `et_nodes` 与 `et_node` 选择
- Android 删除节点同样会清理所有频道的残留配置

### 配置目录
- 桌面端默认迁移到 XDG 标准位置：
  - 配置/身份/频道：`~/.config/asa/`
  - 日志：`~/.local/state/asa/logs/`
  - 下载：`~/Downloads/`
  - 临时文件：`~/.cache/asa/tmp/`
- 首次启动自动从 `~/Documents/ASA/runtime` 复制旧身份、频道和日志；旧文件保留
- `--data-dir <路径>` 仍使用 `<路径>/runtime` 自包含布局，适合便携包和测试
- Windows 使用 `%APPDATA%\asa` / `%LOCALAPPDATA%\asa`，下载为 `%USERPROFILE%\Downloads`
- macOS 使用 `~/Library/Application Support/asa` 与 `~/Library/Logs/asa`

### 验证
- Rust：79 unit + 9 e2e 通过
- Windows 交叉编译通过
- Android：Kotlin 编译通过，arm64 / x86_64 release APK 构建通过
- TUI 实测：多节点添加/选择/移除、延迟测试、邀请码节点传递、旧节点清理
- 节点 `tcp://182.92.90.58:11010` 实测 TCP 22 ms 可达；若仍无法组网，请检查
  `easytier-core` 的 `CAP_NET_ADMIN`（便携版需 `setcap`）

### 兼容性
- 版本号 1.7.4，Android `versionCode 24`
- 配置格式向后兼容；旧频道、旧邀请码可直接使用
- 首次使用标准目录会自动迁移，不需要手动搬文件

### 资产
- Linux：`asa-1.7.4-linux-x86_64.tar.gz`
- Windows：`asa-1.7.4-windows-x86_64.zip`
- Android arm64：`asa-1.7.4-android-arm64.apk`
- Android x86_64：`asa-1.7.4-android-x86_64.apk`
- Arch：`asa-1.7.4-1-x86_64.pkg.tar.zst`
- Debian/Ubuntu：`asa_1.7.4_amd64.deb`
- Fedora/RHEL：`asa-1.7.4-1.x86_64.rpm`

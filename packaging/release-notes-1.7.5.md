ASA v1.7.5：APK 界面/异地连接修复与 TUI 自动提权

### 修复
- APK 频道顶栏显示不完整
  - 顶部只保留「语音房 / 通话 / 更多」三个按钮，成员和邀请码移入「更多」菜单
  - 频道名和副标题限制单行并使用省略号，长频道名/大字体不再挤压图标
- APK 异地连接稳定性
  - 等待 EasyTier 的 VpnService TUN 真正就绪后再用虚拟 IP 重启 ASA，不再固定等 1.5 秒
  - TUN 未就绪时明确报错并回退局域网，不再“看起来已开启但实际连不上”
  - 加入带 `&et=` 的邀请码时强制开启该频道的 EasyTier，避免旧设置导致异地连接失败
  - 聊天页顶部新增 EasyTier 错误横幅和「重连」按钮
  - 设置 → EasyTier 显示运行状态、虚拟 IP、VPN 状态，并提供「重新连接频道」

### 新增
- TUI 自动提权 EasyTier
  - 缺少 TUN 权限时提示：`y` 提权并记住、`o` 仅本次、`n` 不再提示
  - 通过 `pkexec setcap` 弹出系统授权窗口，无需手动敲 sudo（需要 polkit 图形代理）
  - 提权成功后自动重启频道重试；同一进程只尝试一次，避免循环
  - 配置新增：
    - `easytier.auto_elevate`：默认 false；true 时以后直接自动提权
    - `easytier.elevate_prompted`：默认 false；改回 false 可再次看到提示
  - 提权失败或没有 pkexec 时会打印手动 setcap 命令

### 验证
- Rust：79 unit + 9 e2e 通过
- TUI：提权提示、跳过、pkexec 调用、自动提权与避免循环均实测
- Android：Kotlin 编译通过，arm64 / x86_64 release APK 构建通过
- Windows 交叉编译通过

### 兼容性
- 版本号 1.7.5，Android `versionCode 25`
- 配置格式向后兼容；首次运行会补上新的 EasyTier 配置项
- 异地连接建议双方都升级到 1.7.5，并确认邀请码带 `&et=`

### 资产
- Linux：`asa-1.7.5-linux-x86_64.tar.gz`
- Windows：`asa-1.7.5-windows-x86_64.zip`
- Android arm64：`asa-1.7.5-android-arm64.apk`
- Android x86_64：`asa-1.7.5-android-x86_64.apk`
- Arch：`asa-1.7.5-1-x86_64.pkg.tar.zst`
- Debian/Ubuntu：`asa_1.7.5_amd64.deb`
- Fedora/RHEL：`asa-1.7.5-1.x86_64.rpm`

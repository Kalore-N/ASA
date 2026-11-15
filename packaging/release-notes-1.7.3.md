ASA v1.7.3：设置页版本信息与检查更新

### 新增
- Android 设置页「关于」
  - 显示应用版本与 `versionCode`，以及 Rust 核心版本
  - 「检查更新」按钮：连接 GitHub Releases API，手动检测新版本，不后台轮询
  - 发现新版时显示版本号、发布日期和发布说明摘要
  - 「下载并安装」：应用内下载对应 ABI 的 APK，显示下载进度条（已下载 / 总大小 / 百分比）
  - 下载完成后调起系统安装器；未授权时引导到「安装未知应用」设置页
- 下载文件保存在应用私有目录 `filesDir/asa/updates/`，复用现有 FileProvider

### 细节
- 版本比较按数字段处理，支持 `v` 前缀（如 `1.7.10 > 1.7.9`）
- 按设备 ABI 自动选择 `arm64-v8a` / `x86_64` 资产
- GitHub API 频率限制、DNS 失败、连接超时均有中文提示
- 新增 `REQUEST_INSTALL_PACKAGES` 权限

### 验证
- Rust：75 unit + 9 e2e 通过
- Android：Kotlin 编译通过，arm64 / x86_64 release APK 构建通过
- APK 校验：`versionCode 23`、`versionName 1.7.3`，包含 `UpdateChecker` 与安装权限

### 兼容性
- 版本号 1.7.3，Android `versionCode 23`
- 配置格式不变；旧频道、旧邀请码可直接使用
- 检查更新仅访问公开的 GitHub API，不需要登录或 token

### 资产
- Linux：`asa-1.7.3-linux-x86_64.tar.gz`
- Windows：`asa-1.7.3-windows-x86_64.zip`
- Android arm64：`asa-1.7.3-android-arm64.apk`
- Android x86_64：`asa-1.7.3-android-x86_64.apk`
- Arch：`asa-1.7.3-1-x86_64.pkg.tar.zst`
- Debian/Ubuntu：`asa_1.7.3_amd64.deb`
- Fedora/RHEL：`asa-1.7.3-1.x86_64.rpm`

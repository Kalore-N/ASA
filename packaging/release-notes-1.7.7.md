ASA v1.7.7：修复公共下载目录打开

### 修复
- Android 默认下载目录改为公共 `Download/ASA` 后，「打开下载目录」无法打开
  - FileProvider 增加 `Download/ASA` 的外部路径
  - 系统文件管理器可以直接打开该目录
- 1.7.6 的其他修复继续生效：后台更新下载、右侧菜单、新消息顶部提示、
  左右滑动引用、长按标记问题、文件夹发送、通话页面记忆等

### 验证
- Rust：80 unit + 9 e2e 通过
- Android：Kotlin 编译通过，arm64 / x86_64 release APK 构建通过
- Windows 交叉编译通过

### 兼容性
- 版本号 1.7.7，Android `versionCode 27`
- 配置格式不变

### 资产
- Linux：`asa-1.7.7-linux-x86_64.tar.gz`
- Windows：`asa-1.7.7-windows-x86_64.zip`
- Android arm64：`asa-1.7.7-android-arm64.apk`
- Android x86_64：`asa-1.7.7-android-x86_64.apk`
- Arch：`asa-1.7.7-1-x86_64.pkg.tar.zst`
- Debian/Ubuntu：`asa_1.7.7_amd64.deb`
- Fedora/RHEL：`asa-1.7.7-1.x86_64.rpm`

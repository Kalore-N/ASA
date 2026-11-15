ASA v1.7.2：频道删除（Android 群列表）

### 新增
- Android 频道列表支持删除频道
  - 每行右侧新增删除按钮，确认框显示频道名称
  - 默认只删除本机保存的频道配置，不影响其他成员
  - 可勾选「同时删除该频道的聊天记录（不可恢复）」，下载文件始终保留
  - 删除正在连接的频道时会先自动退出（停止节点、前台服务与语音），再删除配置
- 核心新增 `rooms::remove_with_logs`，按频道 ID 精确清理 `logs/asa-<id>-*.jsonl`
  - 不勾选清理时，聊天记录与下载文件都保留

### 修复/改进
- TUI 大厅 `d` 删除频道时，除频道配置外也会清理本机频道设置
  （名称、EasyTier 开关、加入密码），避免重新加入时旧设置残留
- 删除频道不会删除下载目录中的文件

### 验证
- Rust：75 unit + 9 e2e 通过（新增频道删除与日志清理测试）
- Android：Kotlin 编译通过，arm64 / x86_64 release APK 构建通过
- 已在 arm64 真机安装 1.7.2（versionCode 22）

### 兼容性
- 版本号 1.7.2，Android `versionCode 22`
- 配置格式不变；旧频道、旧邀请码可直接使用
- 频道删除只影响本机，不会通知或解散其他成员

### 资产
- Linux：`asa-1.7.2-linux-x86_64.tar.gz`
- Windows：`asa-1.7.2-windows-x86_64.zip`
- Android arm64：`asa-1.7.2-android-arm64.apk`
- Android x86_64：`asa-1.7.2-android-x86_64.apk`
- Arch：`asa-1.7.2-1-x86_64.pkg.tar.zst`
- Debian/Ubuntu：`asa_1.7.2_amd64.deb`
- Fedora/RHEL：`asa-1.7.2-1.x86_64.rpm`

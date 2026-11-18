ASA v1.7.8：修复滑动卡顿与误触引用，调整标记方式

### 修复
- APK 消息列表上下滑动卡顿、偶发误触回复
  - 滑动手势增加方向锁定：纵向滚动不再被消息气泡抢占
  - 只有横向拖动超过阈值才触发回复/标记
  - 拖动时不再每帧启动动画，松手后使用弹簧动画归位
- 点击空白处收起输入法
  - 改为点击输入框和输入法以外的任意区域都清除焦点并收起键盘
  - 点击输入框、发送/麦克风按钮不受影响

### 调整
- 左滑 = 引用回复（保留）
- 右滑 = 标记问题（仅自己的消息）；再次右滑取消标记
- 取消长按消息弹出选项框
- 标记状态本地持久化，别人的消息不能被标记

### 验证
- Rust：80 unit + 9 e2e 通过
- Android：Kotlin 编译通过，arm64 / x86_64 release APK 构建通过
- Windows 交叉编译通过

### 兼容性
- 版本号 1.7.8，Android `versionCode 28`
- 配置格式不变；1.7.6/1.7.7 的下载目录、文件夹发送、后台更新等改动继续生效

### 资产
- Linux：`asa-1.7.8-linux-x86_64.tar.gz`
- Windows：`asa-1.7.8-windows-x86_64.zip`
- Android arm64：`asa-1.7.8-android-arm64.apk`
- Android x86_64：`asa-1.7.8-android-x86_64.apk`
- Arch：`asa-1.7.8-1-x86_64.pkg.tar.zst`
- Debian/Ubuntu：`asa_1.7.8_amd64.deb`
- Fedora/RHEL：`asa-1.7.8-1.x86_64.rpm`

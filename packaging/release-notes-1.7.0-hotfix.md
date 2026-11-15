ASA v1.7.0-hotfix：局域网发现、消息选择与 EasyTier 修复

### 修复
- Android 频道列表不再自动扫描局域网；「局域网频道」移入「加入频道」页，打开时自动扫描并可刷新
- Android 消息支持长按进入系统文本选择，只选择正文；向左滑动消息可回复
- 修复 Android 关闭 EasyTier 后 VPN 通知/图标残留
  - VpnService 被系统 bind 时 `stopService()` 不会触发销毁
  - 现在先关闭 TUN fd、停止 ET 实例，再停止服务
- 修复 Android EasyTier JNI 声明与 2.6.4 原生库不一致
  - `collectNetworkInfos()` 不再传参数，移除不存在的 `listInstances` / `deleteNetworkInstance`
  - 启动后输出 ET 实例、虚拟 IP、peer 和路由信息，便于诊断
- 修复上线同步时重复插入已有消息
  - 本地启动加载的历史消息会写入去重集合
  - Android UI 再按消息 ID 兜底去重
- 修复邀请码不带 EasyTier 节点 / 受邀者不自动添加节点
  - 邀请码携带当前频道节点（ET 未运行也会携带）
  - TUI `join`、Android 加入、局域网加入都会合并 `&et=` 节点
  - 首次加入且频道有节点时默认开启 ET
  - Android 为当前频道开启 ET 时，会把全局节点导入频道配置

### TUI
- 新增 `/mouse [on|off]`
  - `/mouse off` 关闭鼠标捕获，可直接用鼠标拖选文本
  - `/mouse on` 恢复滚轮
  - 鼠标捕获开启时，多数终端也可按住 `Shift` 拖选

### 验证
- Rust：74 unit + 8 e2e 通过
- Android 模拟器：
  - 加入频道页自动发现局域网频道
  - 长按消息进入文本选择，左滑触发回复
  - 开启 ET 后两个 Android 节点通过 ET 虚拟 IP 互通
  - 关闭 ET 后 VPN 网络和图标消失
  - 重复同步后消息不重复

### 兼容性
- 版本号仍为 1.7.0，`versionCode` 20
- 旧配置和旧邀请码仍可用
- TUI 与 APK 使用同一公开节点时，网络名由频道密钥派生；若仍无法互通，请对比两端 `network_name` 是否一致

### 资产
- Linux：`asa-1.7.0-linux-x86_64.tar.gz`
- Windows：`asa-1.7.0-windows-x86_64.zip`
- Android arm64：`asa-1.7.0-android-arm64.apk`
- Android x86_64：`asa-1.7.0-android-x86_64.apk`
- Arch：`asa-1.7.0-1-x86_64.pkg.tar.zst`
- Debian/Ubuntu：`asa_1.7.0_amd64.deb`
- Fedora/RHEL：`asa-1.7.0-1.x86_64.rpm`

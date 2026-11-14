ASA v1.5.0：内嵌 EasyTier 异地组网

### 新增
- 可选 EasyTier 内网穿透：桌面 sidecar、Android JNI + VpnService
- 节点手动导入，以及从 qtet 监控页一键导入未打码节点
- 按频道单独开启/关闭 EasyTier；TUI 大厅按 `n` 添加、`e` 切换；聊天内 `/et`
- 邀请码新增 `&et=` 节点参数；开启后主地址为 EasyTier 虚拟 IP，并保留局域网备用入口
- EasyTier 网络名/密钥由频道密钥派生；成员虚拟 IP 由公钥派生为 `10.144.x.y/16`
- Android per-app VPN：只路由 ASA 自身流量；保留局域网/多中继回退
- EasyTier 启动失败自动回退，不阻塞聊天
- 桌面安装包附带 EasyTier 并在安装时 setcap；Windows/便携版说明管理员权限
- Android 拆分 arm64 / x86_64 APK
- 补充 LGPL-3.0 第三方声明与 EasyTier Android 构建脚本

### 兼容性
- 旧邀请码仍可用；新客户端忽略不认识的参数
- 旧版客户端会忽略 `&et=`，仍可通过局域网/中继加入
- Android 同一用户同时只能有一个 VpnService；工作资料可同时运行两个 VPN

### 资产
- Linux：`asa-1.5.0-linux-x86_64.tar.gz`
- Windows：`asa-1.5.0-windows-x86_64.zip`
- Android arm64：`asa-1.5.0-android-arm64.apk`
- Android x86_64：`asa-1.5.0-android-x86_64.apk`
- Arch：`asa-1.5.0-1-x86_64.pkg.tar.zst`
- Debian/Ubuntu：`asa_1.5.0_amd64.deb`
- Fedora/RHEL：`asa-1.5.0-1.x86_64.rpm`

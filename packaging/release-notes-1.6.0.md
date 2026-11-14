ASA v1.6.0：历史同步、局域网频道与 Vim 浏览

### 修复
- `/sync` 现在从本地全部 JSONL 历史读取，重启后也能同步；日志补齐完整公钥
- 启动时自动加载最近本地历史，TUI/Android 不再从空白开始
- `sync.count` 可在配置文件中自定义，默认 50，0 表示全部（最多 5000 条）
- Android 端补齐同步聊天记录、进行中的传输、打开下载目录
- 修复 Android 设置保存时丢失 EasyTier / sync / TUI 配置字段的问题

### 新增
- TUI 常驻 Vim 浏览模式：`i` 输入、`Esc` 返回、`j/k` 滚动、`g/G` 顶/底
- TUI 鼠标滚轮、PageUp/PageDown、聊天区滚动条和“已向上 N 行”提示
- 配置 `ui.vim_mode`、`ui.scroll_step`、`ui.history_lines`
- 局域网频道发现：宿主 mDNS 广播频道目录
- 创建频道时可设置加入密码；TUI 大厅 `p` 可修改/清除
- 客户端在大厅按 `f` 发现频道，输入密码加入；Android 频道列表顶部同样显示
- 首次加入后保存邀请码和加密后的密码；下次发现频道时大厅自动选中
- 密码使用 PBKDF2-SHA256 + HMAC 挑战应答，邀请码加密传输
- 没有密码的频道以开放形式广播，局域网内可直接加入
- 清理旧发布产物、测试脚本、Android 备份/日志、EasyTier 构建缓存、测试 AVD 和测试 runtime

### 兼容性
- 旧邀请码仍可用
- 旧日志没有完整公钥时，本机消息可显示，其他成员的旧消息无法参与同步
- Android 模拟器之间可能无法互相发现 mDNS，请用两台真机在同一 Wi-Fi 测试

### 资产
- Linux：`asa-1.6.0-linux-x86_64.tar.gz`
- Windows：`asa-1.6.0-windows-x86_64.zip`
- Android arm64：`asa-1.6.0-android-arm64.apk`
- Android x86_64：`asa-1.6.0-android-x86_64.apk`
- Arch：`asa-1.6.0-1-x86_64.pkg.tar.zst`
- Debian/Ubuntu：`asa_1.6.0_amd64.deb`
- Fedora/RHEL：`asa-1.6.0-1.x86_64.rpm`

# 第三方组件

## EasyTier

- 版本：2.6.4
- 许可证：GNU Lesser General Public License v3.0（LGPL-3.0）
- 源码：https://github.com/EasyTier/EasyTier/tree/v2.6.4
- 用途：可选的异地组网/内网穿透层。

ASA 以独立进程（桌面）或独立动态库（Android）的方式使用 EasyTier，
不修改 EasyTier 的源代码。安装包中附带的是 EasyTier 官方发布的
未修改二进制。LGPL-3.0 许可证全文见同目录下的 `EASYTIER_LICENSE`。

如需重新链接或替换 EasyTier：

- 桌面：替换 `easytier-core` / `easytier-cli` 即可；
- Android：替换 APK 中的 `libeasytier_android_jni.so` 与
  `libeasytier_ffi.so`；构建脚本见 `scripts/build-easytier-android.sh`。

EasyTier 的源码、构建脚本和许可证由 EasyTier 项目提供。

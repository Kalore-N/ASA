package com.asa.chat

/**
 * Rust 核心的 JNI 入口。
 *
 * 所有数据都通过 JSON 字符串传递：
 * - start(config)  启动节点
 * - pollEvents()   取出事件数组
 * - sendCommand()  发送命令
 */
object NativeBridge {
    init {
        System.loadLibrary("asa")
    }

    external fun start(config: String): String
    external fun pollEvents(): String
    external fun sendCommand(command: String): String
    external fun groups(dataDir: String): String
    external fun renameGroup(dataDir: String, roomId: String, name: String): String
    external fun setEasytierEnabled(dataDir: String, roomId: String, enabled: Boolean): String
    external fun isRunning(): Boolean
    external fun status(): String
    external fun stop()
    external fun version(): String
}

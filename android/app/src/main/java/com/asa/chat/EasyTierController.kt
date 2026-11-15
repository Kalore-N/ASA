package com.asa.chat

import android.content.Context
import android.content.Intent
import android.net.VpnService
import android.os.Build
import android.util.Log
import com.easytier.jni.EasyTierJNI
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeoutOrNull

/**
 * EasyTier 运行时控制。
 *
 * Android 端的顺序必须是：
 * 1. 启动 EasyTier 实例（固定 ipv4，不依赖 DHCP）；
 * 2. 建立 VpnService TUN，并把 fd 交给 EasyTier；
 * 3. 用虚拟 IP 作为 ASA 的 advertise 重启节点。
 */
object EasyTierController {
    const val INSTANCE = "asa"
    private const val TAG = "AsaEasyTier"

    @Volatile
    var running: Boolean = false
        private set

    @Volatile
    var lastError: String? = null
        private set

    /** 当前 EasyTier 对应的频道；用于关闭频道时精确停止。 */
    @Volatile
    var activeRoom: String? = null
        private set

    /** 最近一次 ET 运行时信息，便于日志和诊断面板。 */
    @Volatile
    var lastInfo: String? = null
        private set

    fun start(
        context: Context,
        network: String,
        secret: String,
        ipv4: String,
        nodes: List<String>,
        roomId: String? = null,
    ): Boolean {
        if (network.isBlank() || secret.isBlank() || ipv4.isBlank() || nodes.isEmpty()) {
            lastError = "EasyTier 配置不完整"
            return false
        }
        // 切频道时先彻底停掉旧实例，避免同名实例仍占用 TUN。
        if (running) stop(context)
        lastError = null
        val config = buildConfig(network, secret, ipv4, nodes)
        Log.i(TAG, "EasyTier config:\n$config")
        return try {
            val parsed = EasyTierJNI.parseConfig(config)
            if (parsed != 0) {
                lastError = EasyTierJNI.getLastError() ?: "配置解析失败"
                return false
            }
            val started = EasyTierJNI.runNetworkInstance(config)
            if (started != 0) {
                lastError = EasyTierJNI.getLastError() ?: "实例启动失败"
                return false
            }
            running = true
            activeRoom = roomId
            lastInfo = null
            val intent = Intent(context, AsaVpnService::class.java)
                .putExtra("ipv4", ipv4.substringBefore('/'))
                .putExtra("prefix", ipv4.substringAfter('/', "16").toIntOrNull() ?: 16)
                .putExtra("instance", INSTANCE)
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                context.startForegroundService(intent)
            } else {
                context.startService(intent)
            }
            Thread {
                for (delay in longArrayOf(2_500, 8_000)) {
                    try {
                        Thread.sleep(delay)
                    } catch (_: InterruptedException) {
                        return@Thread
                    }
                    if (!running) return@Thread
                    val info = runCatching {
                        EasyTierJNI.collectNetworkInfos()
                    }.getOrNull()
                    lastInfo = info
                    Log.i(TAG, "ET status t+${delay}ms $info")
                }
            }.apply {
                name = "asa-et-status"
                isDaemon = true
                start()
            }
            true
        } catch (t: Throwable) {
            lastError = t.message ?: "EasyTier 启动异常"
            Log.e(TAG, "start failed", t)
            false
        }
    }

    /** VPN 被系统撤销或服务销毁时同步运行时状态。 */
    fun markStopped() {
        running = false
        activeRoom = null
        lastInfo = null
    }

    fun stop(context: Context) {
        val wasRunning = running
        running = false
        activeRoom = null
        lastInfo = null
        runCatching { EasyTierJNI.stopAllInstances() }
        AsaVpnService.stopNow(context)
        AsaVpnService.updateStatus("已停止")
        if (wasRunning) Log.i(TAG, "EasyTier stopped")
    }

    /** 通过 ActivityResult 申请系统 VPN 权限，并等待用户确认。 */
    suspend fun ensureVpnPermission(context: Context, timeoutMs: Long = 60_000): Boolean {
        if (VpnService.prepare(context) == null) return true
        val deferred = CompletableDeferred<Boolean>()
        withContext(Dispatchers.Main) {
            VpnPermission.request(context) { granted -> deferred.complete(granted) }
        }
        return withTimeoutOrNull(timeoutMs) { deferred.await() } ?: false
    }

    fun status(): String? {
        return runCatching { EasyTierJNI.collectNetworkInfos() }.getOrNull()
    }

    private fun buildConfig(
        network: String,
        secret: String,
        ipv4: String,
        nodes: List<String>,
    ): String {
        val clean = { value: String -> value.replace("\\", "\\\\").replace("\"", "\\\"") }
        return buildString {
            appendLine("instance_name = \"$INSTANCE\"")
            appendLine("ipv4 = \"${clean(ipv4)}\"")
            appendLine("listeners = [\"tcp://0.0.0.0:11010\", \"udp://0.0.0.0:11010\"]")
            appendLine("mtu = 1380")
            appendLine()
            appendLine("[network_identity]")
            appendLine("network_name = \"${clean(network)}\"")
            appendLine("network_secret = \"${clean(secret)}\"")
            nodes.forEach { node ->
                appendLine()
                appendLine("[[peer]]")
                appendLine("uri = \"${clean(node)}\"")
            }
        }
    }
}

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

    fun start(
        context: Context,
        network: String,
        secret: String,
        ipv4: String,
        nodes: List<String>,
    ): Boolean {
        if (network.isBlank() || secret.isBlank() || ipv4.isBlank() || nodes.isEmpty()) {
            lastError = "EasyTier 配置不完整"
            return false
        }
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
            val intent = Intent(context, AsaVpnService::class.java)
                .putExtra("ipv4", ipv4.substringBefore('/'))
                .putExtra("prefix", ipv4.substringAfter('/', "16").toIntOrNull() ?: 16)
                .putExtra("instance", INSTANCE)
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                context.startForegroundService(intent)
            } else {
                context.startService(intent)
            }
            true
        } catch (t: Throwable) {
            lastError = t.message ?: "EasyTier 启动异常"
            Log.e(TAG, "start failed", t)
            false
        }
    }

    fun stop(context: Context) {
        running = false
        runCatching { EasyTierJNI.stopAllInstances() }
        runCatching { context.stopService(Intent(context, AsaVpnService::class.java)) }
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
        return runCatching { EasyTierJNI.collectNetworkInfos(10) }.getOrNull()
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

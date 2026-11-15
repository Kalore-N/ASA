package com.asa.chat

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Intent
import android.content.pm.ServiceInfo
import android.net.VpnService
import android.os.Build
import android.os.ParcelFileDescriptor
import android.util.Log
import com.easytier.jni.EasyTierJNI
import java.net.InetAddress

/**
 * 只把 EasyTier 虚拟网段路由进 TUN，并且只允许 ASA 自己使用。
 *
 * `addAllowedApplication(packageName)` 是 Android 的 per-app VPN：
 * 其他应用的流量不会进入 ASA 的 TUN。系统仍然只允许一个 VpnService，
 * 因此如果已有其他 VpnService，启动前会由上层提示用户。
 */
class AsaVpnService : VpnService() {
    private var vpnInterface: ParcelFileDescriptor? = null

    override fun onCreate() {
        super.onCreate()
        createChannel()
        instance = this
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent == null) {
            stopSelf()
            return START_NOT_STICKY
        }
        val ipv4 = intent.getStringExtra("ipv4")
        val prefix = intent.getIntExtra("prefix", 16)
        val instance = intent.getStringExtra("instance") ?: EasyTierController.INSTANCE
        if (ipv4.isNullOrBlank()) {
            status = "缺少虚拟 IP"
            stopSelf()
            return START_NOT_STICKY
        }

        startForegroundNotification()
        status = "正在建立 TUN: $ipv4/$prefix"
        return try {
            val builder = Builder()
                .setSession("ASA EasyTier")
                .setMtu(1380)
                .setBlocking(false)
                .addAddress(ipv4, prefix)
                .addRoute(networkAddress(ipv4, prefix), prefix)
            // 只让 ASA 自己走这条 VPN，不影响其他应用。
            runCatching { builder.addAllowedApplication(packageName) }

            val pfd = builder.establish()
            if (pfd == null) {
                status = "establish() 返回 null（VPN 未授权或被占用）"
                Log.e(TAG, status)
                stopSelf()
                return START_NOT_STICKY
            }
            vpnInterface = pfd
            val result = EasyTierJNI.setTunFd(instance, pfd.fd)
            status = if (result == 0) {
                "TUN 已连接: $ipv4/$prefix (fd=${pfd.fd})"
            } else {
                "setTunFd 失败: $result ${EasyTierJNI.getLastError().orEmpty()}"
            }
            Log.i(TAG, status)
            START_NOT_STICKY
        } catch (t: Throwable) {
            status = "建立 TUN 异常: ${t.message}"
            Log.e(TAG, status, t)
            stopSelf()
            START_NOT_STICKY
        }
    }

    override fun onRevoke() {
        status = "VPN 权限被系统撤销"
        cleanup()
        stopSelf()
        super.onRevoke()
    }

    override fun onDestroy() {
        cleanup()
        if (instance === this) instance = null
        super.onDestroy()
    }

    private fun cleanup() {
        runCatching { vpnInterface?.close() }
        vpnInterface = null
        runCatching { EasyTierJNI.stopAllInstances() }
        EasyTierController.markStopped()
        runCatching { stopForeground(STOP_FOREGROUND_REMOVE) }
        status = "已停止"
        Log.i(TAG, "VPN 已清理")
    }

    private fun startForegroundNotification() {
        val contentIntent = PendingIntent.getActivity(
            this,
            0,
            Intent(this, MainActivity::class.java),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )
        val notification: Notification = Notification.Builder(this, CHANNEL_ID)
            .setSmallIcon(android.R.drawable.ic_menu_manage)
            .setContentTitle("ASA EasyTier")
            .setContentText("仅 ASA 使用 EasyTier 内网穿透")
            .setContentIntent(contentIntent)
            .setOngoing(true)
            .build()
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
            startForeground(
                NOTIFICATION_ID,
                notification,
                ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE,
            )
        } else {
            startForeground(NOTIFICATION_ID, notification)
        }
    }

    private fun createChannel() {
        val manager = getSystemService(NotificationManager::class.java)
        manager.createNotificationChannel(
            NotificationChannel(
                CHANNEL_ID,
                "ASA EasyTier",
                NotificationManager.IMPORTANCE_LOW,
            ),
        )
    }

    private fun networkAddress(address: String, prefix: Int): String {
        val bytes = InetAddress.getByName(address).address
        var value = 0L
        for (byte in bytes) {
            value = (value shl 8) or (byte.toLong() and 0xFF)
        }
        val mask = if (prefix == 0) 0L else (0xFFFFFFFFL shl (32 - prefix)) and 0xFFFFFFFFL
        val network = value and mask
        return String.format(
            "%d.%d.%d.%d",
            (network shr 24) and 0xFF,
            (network shr 16) and 0xFF,
            (network shr 8) and 0xFF,
            network and 0xFF,
        )
    }

    companion object {
        private const val TAG = "AsaEasyTier"

        @Volatile
        private var instance: AsaVpnService? = null

        /**
         * VpnService 会被系统 bind，单靠 stopService() 不会触发 onDestroy。
         * 必须先关闭 TUN fd，再 stopSelf，否则系统 VPN 图标会一直残留。
         */
        fun stopNow(context: android.content.Context) {
            val service = instance
            if (service != null) {
                runCatching { service.cleanup() }
                runCatching { service.stopSelf() }
            }
            runCatching {
                context.stopService(Intent(context, AsaVpnService::class.java))
            }
            Log.i(TAG, "VPN stop requested")
        }
        private const val CHANNEL_ID = "asa_easytier_vpn"
        private const val NOTIFICATION_ID = 4570

        @Volatile
        var status: String = "未启动"
            private set

        fun updateStatus(text: String) {
            status = text
        }
    }
}

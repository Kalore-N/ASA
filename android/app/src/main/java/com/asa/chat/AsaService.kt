package com.asa.chat

import android.app.Service
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder

/**
 * 前台服务：只负责让进程保持存活，节点与事件轮询仍由 AsaViewModel 持有。
 * 只有用户在界面里确认「退出频道」时才会被停止。
 */
class AsaService : Service() {
    override fun onCreate() {
        super.onCreate()
        NotificationHelper.createChannels(this)
        val notification = NotificationHelper.backgroundNotification(this)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
            startForeground(
                NotificationHelper.BACKGROUND_ID,
                notification,
                ServiceInfo.FOREGROUND_SERVICE_TYPE_REMOTE_MESSAGING,
            )
        } else {
            startForeground(NotificationHelper.BACKGROUND_ID, notification)
        }
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int =
        START_NOT_STICKY

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onDestroy() {
        stopForeground(STOP_FOREGROUND_REMOVE)
        super.onDestroy()
    }
}

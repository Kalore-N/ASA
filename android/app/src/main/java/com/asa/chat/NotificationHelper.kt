package com.asa.chat

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.os.Build
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat

object NotificationHelper {
    const val CHANNEL_MESSAGES = "asa_messages"
    const val CHANNEL_BACKGROUND = "asa_background"
    const val BACKGROUND_ID = 1001

    fun createChannels(context: Context) {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            val manager = context.getSystemService(NotificationManager::class.java)
            val messages = NotificationChannel(
                CHANNEL_MESSAGES,
                "消息",
                NotificationManager.IMPORTANCE_DEFAULT,
            ).apply {
                description = "频道新消息"
            }
            val background = NotificationChannel(
                CHANNEL_BACKGROUND,
                "后台连接",
                NotificationManager.IMPORTANCE_LOW,
            ).apply {
                description = "保持频道连接"
            }
            manager.createNotificationChannels(listOf(messages, background))
        }
    }

    private fun contentIntent(context: Context): PendingIntent =
        PendingIntent.getActivity(
            context,
            0,
            Intent(context, MainActivity::class.java).apply {
                flags = Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_CLEAR_TOP
            },
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )

    fun backgroundNotification(context: Context): Notification =
        NotificationCompat.Builder(context, CHANNEL_BACKGROUND)
            .setSmallIcon(R.drawable.ic_stat_asa)
            .setContentTitle("ASA 正在保持连接")
            .setContentText("频道在后台运行，点击返回")
            .setOngoing(true)
            .setCategory(NotificationCompat.CATEGORY_SERVICE)
            .setPriority(NotificationCompat.PRIORITY_LOW)
            .setContentIntent(contentIntent(context))
            .build()

    fun notifyMessage(context: Context, nick: String, text: String, mentioned: Boolean) {
        if (AppVisibility.foreground) return
        val title = if (mentioned) "$nick 提到了你" else nick
        val notification = NotificationCompat.Builder(context, CHANNEL_MESSAGES)
            .setSmallIcon(R.drawable.ic_stat_asa)
            .setContentTitle(title)
            .setContentText(text)
            .setStyle(NotificationCompat.BigTextStyle().bigText(text))
            .setAutoCancel(true)
            .setPriority(NotificationCompat.PRIORITY_DEFAULT)
            .setContentIntent(contentIntent(context))
            .build()
        val id = (System.currentTimeMillis() and 0x7fffffff).toInt()
        try {
            NotificationManagerCompat.from(context).notify(id, notification)
        } catch (_: SecurityException) {
            // 用户未授予通知权限时静默忽略。
        }
    }
}

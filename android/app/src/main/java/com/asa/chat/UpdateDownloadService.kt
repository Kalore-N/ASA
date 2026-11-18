package com.asa.chat

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import androidx.core.content.ContextCompat
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch

/** 更新 APK 下载状态；设置页和服务共享。 */
object UpdateDownloadState {
    data class State(
        val running: Boolean = false,
        val done: Long = 0,
        val total: Long = 0,
        val version: String = "",
        val path: String? = null,
        val error: String? = null,
    )

    private val _state = MutableStateFlow(State())
    val state = _state.asStateFlow()

    fun update(value: State) {
        _state.value = value
    }

    fun reset() {
        _state.value = State()
    }
}

/**
 * 应用内更新 APK 的前台下载服务。
 *
 * 设置页只负责发起；下载过程在服务里，切后台、锁屏或离开设置页都不会取消。
 */
class UpdateDownloadService : Service() {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private var job: Job? = null

    override fun onCreate() {
        super.onCreate()
        val manager = getSystemService(NotificationManager::class.java)
        manager.createNotificationChannel(
            NotificationChannel(
                CHANNEL_ID,
                "ASA 更新下载",
                NotificationManager.IMPORTANCE_LOW,
            ),
        )
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent == null) {
            stopSelf()
            return START_NOT_STICKY
        }
        val url = intent.getStringExtra(EXTRA_URL).orEmpty()
        val version = intent.getStringExtra(EXTRA_VERSION).orEmpty()
        if (url.isBlank() || version.isBlank()) {
            stopSelf()
            return START_NOT_STICKY
        }
        startForegroundNotification(version, 0, 0)
        UpdateDownloadState.update(UpdateDownloadState.State(running = true, version = version))
        job?.cancel()
        job = scope.launch {
            runCatching {
                UpdateChecker.downloadUrl(this@UpdateDownloadService, url, version) { _, done, total ->
                    UpdateDownloadState.update(
                        UpdateDownloadState.State(
                            running = true,
                            done = done,
                            total = total,
                            version = version,
                        ),
                    )
                    notifyProgress(version, done, total)
                }
            }.onSuccess { file ->
                UpdateDownloadState.update(
                    UpdateDownloadState.State(
                        running = false,
                        done = file.length(),
                        total = file.length(),
                        version = version,
                        path = file.absolutePath,
                    ),
                )
                notifyDone(version, file)
                stopForeground(STOP_FOREGROUND_DETACH)
                stopSelf()
            }.onFailure { error ->
                UpdateDownloadState.update(
                    UpdateDownloadState.State(
                        running = false,
                        version = version,
                        error = error.message ?: "下载失败",
                    ),
                )
                notifyFailed(version, error.message ?: "下载失败")
                stopForeground(STOP_FOREGROUND_DETACH)
                stopSelf()
            }
        }
        return START_NOT_STICKY
    }

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onDestroy() {
        job?.cancel()
        scope.cancel()
        super.onDestroy()
    }

    private fun startForegroundNotification(version: String, done: Long, total: Long) {
        val notification = buildNotification(
            version,
            done,
            total,
            finished = total > 0 && done >= total,
        )
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            startForeground(
                NOTIFICATION_ID,
                notification,
                ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC,
            )
        } else {
            startForeground(NOTIFICATION_ID, notification)
        }
    }

    private fun notifyProgress(version: String, done: Long, total: Long) {
        val manager = getSystemService(NotificationManager::class.java)
        manager.notify(
            NOTIFICATION_ID,
            buildNotification(version, done, total, finished = false),
        )
    }

    private fun notifyDone(version: String, file: java.io.File) {
        val manager = getSystemService(NotificationManager::class.java)
        val intent = Intent(this, MainActivity::class.java).apply {
            putExtra(EXTRA_INSTALL_PATH, file.absolutePath)
            addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP)
        }
        val pending = PendingIntent.getActivity(
            this,
            1,
            intent,
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )
        val notification = Notification.Builder(this, CHANNEL_ID)
            .setSmallIcon(android.R.drawable.stat_sys_download_done)
            .setContentTitle("ASA $version 下载完成")
            .setContentText("点击打开应用并安装")
            .setContentIntent(pending)
            .setAutoCancel(true)
            .build()
        manager.notify(NOTIFICATION_ID + 1, notification)
    }

    private fun notifyFailed(version: String, message: String) {
        val manager = getSystemService(NotificationManager::class.java)
        val notification = Notification.Builder(this, CHANNEL_ID)
            .setSmallIcon(android.R.drawable.stat_notify_error)
            .setContentTitle("ASA $version 下载失败")
            .setContentText(message)
            .setAutoCancel(true)
            .build()
        manager.notify(NOTIFICATION_ID + 2, notification)
    }

    private fun buildNotification(
        version: String,
        done: Long,
        total: Long,
        finished: Boolean,
    ): Notification {
        val builder = Notification.Builder(this, CHANNEL_ID)
            .setSmallIcon(android.R.drawable.stat_sys_download)
            .setContentTitle("正在下载 ASA $version")
            .setOngoing(!finished)
        if (total > 0) {
            val percent = ((done * 100) / total).toInt().coerceIn(0, 100)
            builder.setContentText("$percent%")
            builder.setProgress(100, percent, false)
        } else {
            builder.setContentText("已下载 ${done / 1024 / 1024} MB")
            builder.setProgress(0, 0, true)
        }
        return builder.build()
    }

    companion object {
        private const val CHANNEL_ID = "asa_update"
        private const val NOTIFICATION_ID = 4580
        const val EXTRA_URL = "url"
        const val EXTRA_VERSION = "version"
        const val EXTRA_INSTALL_PATH = "install_path"

        fun start(context: Context, url: String, version: String) {
            val intent = Intent(context, UpdateDownloadService::class.java).apply {
                putExtra(EXTRA_URL, url)
                putExtra(EXTRA_VERSION, version)
            }
            ContextCompat.startForegroundService(context, intent)
        }
    }
}

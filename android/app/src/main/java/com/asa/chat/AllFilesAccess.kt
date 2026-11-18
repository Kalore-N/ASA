package com.asa.chat

import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.Environment
import android.provider.Settings
import java.io.File

/**
 * Android 11+ 的“所有文件访问”权限。
 *
 * ASA 需要把接收文件直接写到公共 `Download/ASA`，Rust 侧使用普通 File API；
 * 未授权时回退到应用私有目录，避免写入失败。
 */
object AllFilesAccess {
    fun supported(): Boolean = Build.VERSION.SDK_INT >= Build.VERSION_CODES.R

    fun has(context: Context): Boolean {
        if (!supported()) return false
        return Environment.isExternalStorageManager()
    }

    fun publicDownloadDir(): File =
        File(Environment.getExternalStoragePublicDirectory(Environment.DIRECTORY_DOWNLOADS), "ASA")

    fun shouldAsk(context: Context): Boolean {
        if (!supported() || has(context)) return false
        val prefs = context.getSharedPreferences("asa_prefs", Context.MODE_PRIVATE)
        return !prefs.getBoolean("asked_all_files", false)
    }

    fun markAsked(context: Context) {
        context.getSharedPreferences("asa_prefs", Context.MODE_PRIVATE)
            .edit()
            .putBoolean("asked_all_files", true)
            .apply()
    }

    fun request(context: Context) {
        markAsked(context)
        if (!supported()) return
        val intent = Intent(
            Settings.ACTION_MANAGE_APP_ALL_FILES_ACCESS_PERMISSION,
            Uri.parse("package:${context.packageName}"),
        ).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        val fallback = Intent(Settings.ACTION_MANAGE_ALL_FILES_ACCESS_PERMISSION)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        runCatching { context.startActivity(intent) }
            .onFailure { runCatching { context.startActivity(fallback) } }
    }
}

package com.asa.chat

import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.provider.Settings
import androidx.core.content.FileProvider
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.withContext
import org.json.JSONArray
import org.json.JSONObject
import java.io.File
import java.net.HttpURLConnection
import java.net.URL

/** GitHub 上的一个可用更新。 */
data class UpdateInfo(
    val version: String,
    val tag: String,
    val name: String,
    val notes: String,
    val pageUrl: String,
    val apkUrl: String?,
    val publishedAt: String,
)

/**
 * 通过 GitHub Releases API 检查更新，并在应用内下载 APK。
 *
 * 只在设置页手动触发，不自动联网。仓库是公开的，未认证请求有频率限制。
 */
object UpdateChecker {
    private const val REPO = "Kalore-N/ASA"
    private const val LATEST_URL = "https://api.github.com/repos/$REPO/releases/latest"
    private const val USER_AGENT = "ASA-Android"

    suspend fun check(): Result<UpdateInfo> = withContext(Dispatchers.IO) {
        runCatching {
            val text = httpGet(LATEST_URL, "application/vnd.github+json")
            val obj = JSONObject(text)
            val tag = obj.optString("tag_name").trim()
            if (tag.isBlank()) error("GitHub 返回内容缺少版本号")
            val version = tag.trimStart('v', 'V')
            val asset = selectApk(obj.optJSONArray("assets"))
            UpdateInfo(
                version = version,
                tag = tag,
                name = obj.optString("name").ifBlank { tag },
                notes = obj.optString("body"),
                pageUrl = obj.optString("html_url")
                    .ifBlank { "https://github.com/$REPO/releases/latest" },
                apkUrl = asset,
                publishedAt = obj.optString("published_at"),
            )
        }
    }

    /** 远程版本是否比当前版本新；按数字段比较，忽略 v 前缀。 */
    fun isNewer(remote: String, current: String): Boolean {
        fun parts(value: String): List<Int> = value
            .trim()
            .trimStart('v', 'V')
            .split('.', '-', '+')
            .mapNotNull { part -> part.takeWhile(Char::isDigit).toIntOrNull() }
        val left = parts(remote)
        val right = parts(current)
        for (index in 0 until maxOf(left.size, right.size)) {
            val a = left.getOrElse(index) { 0 }
            val b = right.getOrElse(index) { 0 }
            if (a != b) return a > b
        }
        return false
    }

    /**
     * 下载 APK 到 `filesDir/asa/updates/`，[onProgress] 在 IO 线程回调。
     * [onProgress] 参数：是否未知总大小、已下载字节、总字节（0 表示未知）。
     */
    suspend fun download(
        context: Context,
        info: UpdateInfo,
        onProgress: (Boolean, Long, Long) -> Unit,
    ): File = withContext(Dispatchers.IO) {
        val url = info.apkUrl ?: error("该版本没有可用的 APK 资产")
        val dir = File(context.filesDir, "asa/updates").apply { mkdirs() }
        val file = File(dir, "asa-${info.version}-android.apk")
        val conn = (URL(url).openConnection() as HttpURLConnection).apply {
            connectTimeout = 15_000
            readTimeout = 30_000
            instanceFollowRedirects = true
            setRequestProperty("User-Agent", USER_AGENT)
        }
        try {
            val code = conn.responseCode
            if (code !in 200..299) error("下载失败：HTTP $code")
            val total = conn.contentLengthLong.coerceAtLeast(0)
            onProgress(total <= 0, 0, total)
            conn.inputStream.use { input ->
                file.outputStream().use { output ->
                    val buffer = ByteArray(64 * 1024)
                    var done = 0L
                    var lastTick = 0L
                    while (true) {
                        ensureActive()
                        val read = input.read(buffer)
                        if (read < 0) break
                        output.write(buffer, 0, read)
                        done += read
                        val now = System.currentTimeMillis()
                        if (now - lastTick >= 200) {
                            lastTick = now
                            onProgress(total <= 0, done, total)
                        }
                    }
                    onProgress(total <= 0, done, total)
                }
            }
        } finally {
            conn.disconnect()
        }
        file
    }

    /** 是否已允许安装未知来源应用。 */
    fun canInstall(context: Context): Boolean =
        Build.VERSION.SDK_INT < Build.VERSION_CODES.O ||
            context.packageManager.canRequestPackageInstalls()

    /** 跳转到本应用的「安装未知应用」授权页。 */
    fun requestInstallPermission(context: Context) {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O) return
        val intent = Intent(
            Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES,
            Uri.parse("package:${context.packageName}"),
        ).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        runCatching { context.startActivity(intent) }
    }

    /** 调起系统安装器。 */
    fun install(context: Context, file: File) {
        val uri = FileProvider.getUriForFile(
            context,
            "${context.packageName}.fileprovider",
            file,
        )
        val intent = Intent(Intent.ACTION_VIEW).apply {
            setDataAndType(uri, "application/vnd.android.package-archive")
            addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
            addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        }
        context.startActivity(intent)
    }

    fun openReleasePage(context: Context, url: String) {
        val intent = Intent(Intent.ACTION_VIEW, Uri.parse(url))
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        runCatching { context.startActivity(intent) }
    }

    private fun selectApk(assets: JSONArray?): String? {
        if (assets == null) return null
        val candidates = buildList {
            for (index in 0 until assets.length()) {
                val item = assets.optJSONObject(index) ?: continue
                val name = item.optString("name")
                val url = item.optString("browser_download_url")
                if (name.endsWith(".apk", ignoreCase = true) && url.isNotBlank()) {
                    add(name to url)
                }
            }
        }
        if (candidates.isEmpty()) return null
        val abi = Build.SUPPORTED_ABIS.firstOrNull().orEmpty()
        val wanted = when {
            abi.contains("arm64") -> "arm64"
            abi.contains("x86_64") -> "x86_64"
            else -> ""
        }
        return candidates.firstOrNull { it.first.contains(wanted, ignoreCase = true) }?.second
            ?: candidates.first().second
    }

    private fun httpGet(url: String, accept: String): String {
        val conn = (URL(url).openConnection() as HttpURLConnection).apply {
            connectTimeout = 10_000
            readTimeout = 15_000
            instanceFollowRedirects = true
            setRequestProperty("Accept", accept)
            setRequestProperty("User-Agent", USER_AGENT)
        }
        try {
            val code = conn.responseCode
            if (code !in 200..299) {
                // 403/429 通常是未认证请求触发了 GitHub 频率限制。
                if (code == 403 || code == 429) {
                    error("GitHub API 请求过于频繁，请稍后再试（HTTP $code）")
                }
                error("GitHub 请求失败：HTTP $code")
            }
            return conn.inputStream.bufferedReader().use { it.readText() }
        } finally {
            conn.disconnect()
        }
    }
}

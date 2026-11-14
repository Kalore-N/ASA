package com.asa.chat

import android.content.Context
import org.json.JSONArray
import org.json.JSONObject
import java.io.File

data class AppConfig(
    val downloadDir: String = "",
    val maxFileMb: Long = 100,
    val autoAccept: Boolean = false,
    val theme: String = "dark",
    val fontSize: Int = 15,
    val compact: Boolean = false,
    val mdns: Boolean = true,
    val relays: List<String> = emptyList(),
    val port: Int = 7777,
)

/**
 * 与 Rust 共用 `<filesDir>/asa/runtime/data/config.json`。
 * Rust 启动时读取同一份文件，因此桌面/Android 的字段含义一致。
 */
object ConfigStore {
    fun path(context: Context): File =
        File(context.filesDir, "asa/runtime/data/config.json")

    fun defaultDownloadDir(context: Context): String =
        File(context.filesDir, "asa/runtime/downloads").absolutePath

    fun load(context: Context): AppConfig {
        val file = path(context)
        if (!file.exists()) return AppConfig()
        return runCatching {
            val json = JSONObject(file.readText())
            val ui = json.optJSONObject("ui") ?: JSONObject()
            val net = json.optJSONObject("network") ?: JSONObject()
            val relays = net.optJSONArray("relays")?.let { array ->
                (0 until array.length())
                    .map { array.optString(it) }
                    .filter { it.isNotBlank() }
            }.orEmpty()
            val rawDownload = json.optString("download_dir")
            AppConfig(
                downloadDir = rawDownload.takeIf {
                    it.isNotBlank() && it != "null"
                }.orEmpty(),
                maxFileMb = json.optLong("max_file_mb", 100),
                autoAccept = json.optBoolean("auto_accept", false),
                theme = ui.optString("theme", "dark"),
                fontSize = ui.optInt("font_size", 15).coerceIn(10, 30),
                compact = ui.optBoolean("compact", false),
                mdns = net.optBoolean("mdns", true),
                relays = relays,
                port = net.optInt("port", 7777),
            )
        }.getOrElse { AppConfig() }
    }

    fun save(context: Context, config: AppConfig) {
        val file = path(context)
        file.parentFile?.mkdirs()
        val json = JSONObject().apply {
            put("version", 1)
            put(
                "download_dir",
                if (config.downloadDir.isBlank()) JSONObject.NULL else config.downloadDir,
            )
            put("max_file_mb", config.maxFileMb)
            put("auto_accept", config.autoAccept)
            put("ui", JSONObject().apply {
                put("theme", config.theme)
                put("font_size", config.fontSize)
                put("compact", config.compact)
            })
            put("network", JSONObject().apply {
                put("port", config.port)
                put("mdns", config.mdns)
                put("relays", JSONArray(config.relays))
            })
        }
        file.writeText(json.toString(2))
    }
}

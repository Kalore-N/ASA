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
    val voiceEnabled: Boolean = true,
    val voicePushToTalk: Boolean = false,
    val voiceRecordDir: String = "",
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

    fun defaultJson(): JSONObject = JSONObject().apply {
        put("version", 2)
        put("download_dir", JSONObject.NULL)
        put("max_file_mb", 100)
        put("auto_accept", false)
        put("ui", JSONObject().apply {
            put("theme", "dark")
            put("font_size", 15)
            put("compact", false)
            put("history_lines", 200)
            put("vim_mode", true)
            put("scroll_step", 3)
        })
        put("network", JSONObject().apply {
            put("port", 7777)
            put("mdns", true)
            put("relays", JSONArray())
        })
        put("sync", JSONObject().apply {
            put("enabled", true)
            put("count", 50)
        })
        put("easytier", JSONObject().apply {
            put("nodes", JSONArray())
            put("auto_enable", true)
        })
        put("voice", JSONObject().apply {
            put("enabled", true)
            put("codec", "pcm")
            put("sample_rate", 16000)
            put("channels", 1)
            put("frame_ms", 20)
            put("push_to_talk", false)
            put("record_dir", JSONObject.NULL)
            put("input_device", JSONObject.NULL)
            put("output_device", JSONObject.NULL)
        })
    }

    private fun mergeMissing(target: JSONObject, defaults: JSONObject) {
        for (key in defaults.keys()) {
            val fallback = defaults.opt(key)
            val current = target.opt(key)
            if (current == null || current == JSONObject.NULL) {
                target.put(key, fallback)
            } else if (current is JSONObject && fallback is JSONObject) {
                mergeMissing(current, fallback)
            }
        }
    }

    /** 补齐缺失字段并写回；保留未知字段。 */
    fun ensureFull(context: Context) {
        val file = path(context)
        val existing = runCatching {
            if (file.exists()) JSONObject(file.readText()) else JSONObject()
        }.getOrElse { return }
        mergeMissing(existing, defaultJson())
        existing.put("version", 2)
        file.parentFile?.mkdirs()
        file.writeText(existing.toString(2))
    }

    fun load(context: Context): AppConfig {
        ensureFull(context)
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
                voiceEnabled = json.optJSONObject("voice")?.optBoolean("enabled", true) ?: true,
                voicePushToTalk = json.optJSONObject("voice")?.optBoolean("push_to_talk", false) ?: false,
                voiceRecordDir = json.optJSONObject("voice")?.optString("record_dir")
                    ?.takeUnless { it == "null" }
                    .orEmpty(),
            )
        }.getOrElse { AppConfig() }
    }

    fun save(context: Context, config: AppConfig) {
        val file = path(context)
        file.parentFile?.mkdirs()
        // 保留 Rust 侧的 easytier / sync 等字段，只更新界面设置的字段。
        val json = runCatching {
            if (file.exists()) JSONObject(file.readText()) else JSONObject()
        }.getOrDefault(JSONObject())
        mergeMissing(json, defaultJson())
        json.put("version", 2)
        json.put(
            "download_dir",
            if (config.downloadDir.isBlank()) JSONObject.NULL else config.downloadDir,
        )
        json.put("max_file_mb", config.maxFileMb)
        json.put("auto_accept", config.autoAccept)
        val ui = json.optJSONObject("ui") ?: JSONObject()
        ui.put("theme", config.theme)
        ui.put("font_size", config.fontSize)
        ui.put("compact", config.compact)
        json.put("ui", ui)
        json.put("network", JSONObject().apply {
            put("port", config.port)
            put("mdns", config.mdns)
            put("relays", JSONArray(config.relays))
        })
        val voice = json.optJSONObject("voice") ?: JSONObject()
        voice.put("enabled", config.voiceEnabled)
        voice.put("push_to_talk", config.voicePushToTalk)
        voice.put(
            "record_dir",
            if (config.voiceRecordDir.isBlank()) JSONObject.NULL else config.voiceRecordDir,
        )
        json.put("voice", voice)
        file.writeText(json.toString(2))
    }
}

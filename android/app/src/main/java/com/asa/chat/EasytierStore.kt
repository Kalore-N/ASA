package com.asa.chat

import android.content.Context
import org.json.JSONArray
import org.json.JSONObject
import java.io.File
import java.net.HttpURLConnection
import java.net.URL

/**
 * EasyTier 节点列表的本地存储。
 *
 * 节点保存在 Rust 的全局配置 `filesDir/asa/data/config.json` 的
 * `easytier.nodes` 数组里；这里只做最小 JSON 读写，保留其他字段。
 */
object EasytierStore {
    private const val QTET_URL = "https://info.qtet.cn/uptime/status/easytier"
    private val NODE_REGEX =
        Regex("""(?:tcp|udp|ws|wss|quic)://[A-Za-z0-9._~:/?#@!$&'()*+,;=%-]+""")

    private fun configFile(context: Context): File {
        return File(File(context.filesDir, "asa"), "runtime/data/config.json")
    }

    fun isValidNode(text: String): Boolean {
        val value = text.trim()
        if (value.isEmpty() || value.contains('*')) return false
        if (value.any { it.isWhitespace() || it.isISOControl() }) return false
        val (scheme, rest) = if (value.contains("://")) {
            value.substringBefore("://").lowercase() to value.substringAfter("://")
        } else {
            "tcp" to value
        }
        if (scheme !in setOf("tcp", "udp", "ws", "wss", "quic", "wg", "faketcp")) return false
        if (rest.isBlank()) return false
        return rest.contains(':') || scheme == "ws" || scheme == "wss"
    }

    fun load(context: Context): List<String> {
        ConfigStore.ensureFull(context)
        val file = configFile(context)
        if (!file.exists()) return emptyList()
        return runCatching {
            val root = JSONObject(file.readText())
            val array = root.optJSONObject("easytier")?.optJSONArray("nodes") ?: return emptyList()
            buildList {
                for (index in 0 until array.length()) {
                    val node = array.optString(index).trim()
                    if (node.isNotEmpty()) add(node)
                }
            }
        }.getOrDefault(emptyList())
    }

    fun add(context: Context, node: String): Boolean {
        val value = node.trim()
        if (!isValidNode(value)) return false
        val nodes = load(context).toMutableList()
        if (nodes.contains(value)) return true
        nodes.add(value)
        save(context, nodes)
        return true
    }

    fun remove(context: Context, node: String) {
        val nodes = load(context).filter { it != node }
        save(context, nodes)
    }

    fun save(context: Context, nodes: List<String>) {
        ConfigStore.ensureFull(context)
        val file = configFile(context)
        val root = runCatching {
            if (file.exists()) JSONObject(file.readText()) else JSONObject()
        }.getOrDefault(JSONObject())
        if (!root.has("version")) root.put("version", 1)
        val easytier = root.optJSONObject("easytier") ?: JSONObject()
        easytier.put("nodes", JSONArray(nodes.distinct()))
        root.put("easytier", easytier)
        file.parentFile?.mkdirs()
        file.writeText(root.toString(2))
    }

    /** 用 TCP 连接测试节点；返回毫秒延迟。udp/quic 无法用 TCP 握手判断。 */
    fun probe(node: String, timeoutMs: Int = 2500): Result<Long> = runCatching {
        val value = node.trim()
        if (!isValidNode(value)) error("节点地址无效")
        val scheme = if (value.contains("://")) {
            value.substringBefore("://").lowercase()
        } else {
            "tcp"
        }
        if (scheme !in setOf("tcp", "ws", "wss", "faketcp")) {
            error("$scheme 节点不支持 TCP 探测")
        }
        val rest = if (value.contains("://")) {
            value.substringAfter("://").trimEnd('/')
        } else {
            value
        }
        val host: String
        val port: Int
        if (rest.startsWith("[")) {
            val end = rest.indexOf(']')
            if (end <= 0) error("IPv6 地址格式错误")
            host = rest.substring(1, end)
            port = rest.substring(end + 1).removePrefix(":").toIntOrNull()
                ?: error("端口无效")
        } else {
            val index = rest.lastIndexOf(':')
            if (index <= 0) error("节点缺少端口")
            host = rest.substring(0, index)
            port = rest.substring(index + 1).toIntOrNull() ?: error("端口无效")
        }
        val started = System.nanoTime()
        java.net.Socket().use { socket ->
            socket.connect(java.net.InetSocketAddress(host, port), timeoutMs)
        }
        (System.nanoTime() - started) / 1_000_000
    }

    /** 从 qtet 监控页抓取未打码节点。只做纯文本正则提取，不执行页面脚本。 */
    fun fetchQtetNodes(): Result<List<String>> = runCatching {
        val connection = (URL(QTET_URL).openConnection() as HttpURLConnection).apply {
            connectTimeout = 15_000
            readTimeout = 15_000
            requestMethod = "GET"
            setRequestProperty("User-Agent", "ASA-Android/1.5")
        }
        try {
            val code = connection.responseCode
            if (code !in 200..299) {
                error("监控页返回 HTTP $code")
            }
            val text = connection.inputStream.bufferedReader().use { it.readText() }
            NODE_REGEX.findAll(text)
                .map { it.value.trimEnd('/', '.', ',', ';', ')', ']') }
                .filter { it.isNotBlank() && !it.contains('*') && isValidNode(it) }
                .distinct()
                .take(64)
                .toList()
        } finally {
            connection.disconnect()
        }
    }

    /** 导入所有未打码节点，返回新增数量。 */
    fun importQtet(context: Context): Result<Int> {
        return fetchQtetNodes().map { fetched ->
            val nodes = load(context).toMutableList()
            var added = 0
            for (node in fetched) {
                if (!nodes.contains(node)) {
                    nodes.add(node)
                    added++
                }
            }
            if (added > 0) save(context, nodes)
            added
        }
    }
}

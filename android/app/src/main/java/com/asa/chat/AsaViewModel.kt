package com.asa.chat

import android.app.Application
import android.content.Intent
import android.net.Uri
import android.provider.OpenableColumns
import androidx.core.content.ContextCompat
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import org.json.JSONArray
import org.json.JSONObject
import java.io.File

class AsaViewModel(application: Application) : AndroidViewModel(application) {

    private val _state = MutableStateFlow(UiState())
    val state: StateFlow<UiState> = _state.asStateFlow()

    private val pollJob: Job = viewModelScope.launch(Dispatchers.IO) {
        while (isActive) {
            val json = runCatching { NativeBridge.pollEvents() }.getOrElse { "[]" }
            if (json.isNotBlank() && json != "[]") {
                runCatching { handleEvents(json) }
            }
            delay(180)
        }
    }

    private var nextKey = 0L
    private var lastConfig: AppConfig? = null

    init {
        val app = getApplication<Application>()
        if (runCatching { NativeBridge.isRunning() }.getOrDefault(false)) {
            runCatching {
                val obj = JSONObject(NativeBridge.status())
                if (obj.optBoolean("ok")) {
                    _state.value = UiState(
                        screen = Screen.Chat,
                        nick = obj.optString("nick"),
                        me = obj.optString("pubkey"),
                        roomId = obj.optString("room_id"),
                        roomName = obj.optString("room_name").takeUnless { it == "null" }.orEmpty(),
                        invite = obj.optString("invite"),
                        listen = obj.optString("advertise"),
                        downloads = obj.optString("downloads"),
                        groups = parseGroups(obj.optJSONArray("groups")),
                    )
                }
            }
        } else {
            reloadGroups(app, updateScreen = true)
        }
    }

    private fun dataDir(app: Application): String = File(app.filesDir, "asa").absolutePath

    private fun parseGroups(array: JSONArray?): List<SavedGroup> {
        if (array == null) return emptyList()
        val groups = mutableListOf<SavedGroup>()
        for (index in 0 until array.length()) {
            val item = array.optJSONObject(index) ?: continue
            val id = item.optString("id")
            if (id.isBlank()) continue
            groups += SavedGroup(
                id = id,
                name = item.optString("name").ifBlank { "频道 ${id.take(8)}" },
                lastUsed = item.optLong("last_used"),
                active = item.optBoolean("active"),
            )
        }
        return groups
    }

    private fun reloadGroups(app: Application, updateScreen: Boolean = false) {
        val groups = runCatching {
            val result = JSONObject(NativeBridge.groups(dataDir(app)))
            parseGroups(result.optJSONArray("groups"))
        }.getOrDefault(emptyList())
        _state.update { current ->
            current.copy(
                groups = groups,
                screen = if (updateScreen && current.screen == Screen.Connect) {
                    if (groups.isEmpty()) Screen.Connect else Screen.Groups
                } else {
                    current.screen
                },
            )
        }
    }

    fun refreshGroups() {
        reloadGroups(getApplication())
    }

    fun showConnect() {
        _state.update { it.copy(screen = Screen.Connect) }
    }

    fun showGroups() {
        _state.update { it.copy(screen = Screen.Groups) }
        refreshGroups()
    }

    fun enterGroup(roomId: String) {
        if (roomId == _state.value.roomId &&
            runCatching { NativeBridge.isRunning() }.getOrDefault(false)
        ) {
            _state.update { it.copy(screen = Screen.Chat) }
            return
        }
        val app = getApplication<Application>()
        val config = lastConfig ?: ConfigStore.load(app)
        start(
            mode = "chat",
            nick = _state.value.nick,
            invite = null,
            portText = config.port.toString(),
            connectText = "",
            appConfig = config,
            roomId = roomId,
        )
    }

    fun start(
        mode: String,
        nick: String,
        invite: String?,
        portText: String,
        connectText: String,
        appConfig: AppConfig,
        roomId: String? = null,
    ) {
        if (_state.value.connecting) return
        _state.update { it.copy(connecting = true, error = null, info = null) }
        viewModelScope.launch(Dispatchers.IO) {
            val app = getApplication<Application>()
            val dataDir = File(app.filesDir, "asa")
            val port = portText.trim().toIntOrNull()?.takeIf { it in 1024..65535 }
                ?: appConfig.port
            val savedConfig = appConfig.copy(port = port)
            lastConfig = savedConfig
            ConfigStore.save(app, savedConfig)
            val connects = connectText
                .split(',', '\n')
                .map { it.trim() }
                .filter { it.isNotEmpty() }

            val config = JSONObject().apply {
                put("data_dir", dataDir.absolutePath)
                put("mode", mode)
                put("nick", nick.trim())
                put("port", port)
                put("mdns", appConfig.mdns)
                put("max_file_mb", appConfig.maxFileMb)
                put("auto_accept", appConfig.autoAccept)
                if (appConfig.downloadDir.isNotBlank()) {
                    put("download_dir", appConfig.downloadDir)
                }
                if (appConfig.relays.isNotEmpty()) {
                    put("relays", JSONArray(appConfig.relays))
                }
                if (!invite.isNullOrBlank()) put("invite", invite.trim())
                if (!roomId.isNullOrBlank()) put("room_id", roomId)
                put("connect", JSONArray(connects))
            }.toString()

            val result = runCatching { NativeBridge.start(config) }.getOrElse { error ->
                JSONObject()
                    .put("ok", false)
                    .put("error", error.message ?: "启动失败")
                    .toString()
            }
            val obj = runCatching { JSONObject(result) }.getOrNull()
            if (obj?.optBoolean("ok") == true) {
                ContextCompat.startForegroundService(
                    app,
                    Intent(app, AsaService::class.java),
                )
                _state.update {
                    it.copy(
                        screen = Screen.Chat,
                        connecting = false,
                        error = null,
                        nick = obj.optString("nick", nick),
                        me = obj.optString("pubkey"),
                        roomId = obj.optString("room_id"),
                        roomName = obj.optString("room_name").takeUnless { it == "null" }.orEmpty(),
                        invite = obj.optString("invite"),
                        listen = obj.optString("advertise"),
                        downloads = obj.optString("downloads"),
                        groups = parseGroups(obj.optJSONArray("groups")),
                        messages = emptyList(),
                        members = emptyList(),
                        offers = emptyList(),
                        transfers = emptyList(),
                        host = null,
                        replyTarget = null,
                    )
                }
            } else {
                _state.update {
                    it.copy(
                        connecting = false,
                        error = obj?.optString("error").orEmpty().ifBlank { "启动失败" },
                    )
                }
            }
        }
    }

    fun send(text: String) {
        val trimmed = text.trim()
        if (trimmed.isEmpty()) return
        val target = _state.value.replyTarget
        val command = JSONObject().apply {
            put("cmd", "chat")
            put("text", trimmed)
            if (target != null) {
                put("reply", JSONObject().apply {
                    put("id", target.id)
                    put("nick", target.nick)
                    put("preview", preview(target.text))
                })
            }
        }
        val result = runCatching { NativeBridge.sendCommand(command.toString()) }
            .getOrElse { return }
        if (result.contains("\"ok\":false")) {
            _state.update { it.copy(error = "发送失败") }
            return
        }
        _state.update { it.copy(replyTarget = null) }
    }

    fun setReply(message: ChatMessage) {
        if (message.system || message.id.isBlank()) return
        _state.update { it.copy(replyTarget = message) }
    }

    fun clearReply() {
        _state.update { it.copy(replyTarget = null) }
    }

    fun setRoomName(name: String) {
        val trimmed = name.trim()
        if (trimmed.isEmpty()) return
        val command = JSONObject().put("cmd", "set_room_name").put("name", trimmed)
        NativeBridge.sendCommand(command.toString())
        _state.update { current ->
            current.copy(
                roomName = trimmed,
                groups = current.groups.map { group ->
                    if (group.id == current.roomId) group.copy(name = trimmed) else group
                },
            )
        }
    }

    fun setNick(nick: String) {
        val trimmed = nick.trim()
        if (trimmed.isEmpty()) return
        val command = JSONObject().put("cmd", "set_nick").put("nick", trimmed)
        NativeBridge.sendCommand(command.toString())
    }

    fun connect(addr: String) {
        val trimmed = addr.trim()
        if (trimmed.isEmpty()) return
        val command = JSONObject().put("cmd", "connect").put("addr", trimmed)
        NativeBridge.sendCommand(command.toString())
        _state.update { it.copy(info = "正在连接 $trimmed") }
    }

    fun sendFile(uri: Uri, targets: List<Member>) {
        if (targets.isEmpty()) return
        viewModelScope.launch(Dispatchers.IO) {
            val app = getApplication<Application>()
            try {
                val name = displayName(uri) ?: "file.bin"
                val dir = File(app.cacheDir, "outgoing/${System.currentTimeMillis()}").apply {
                    mkdirs()
                }
                val safe = name.replace('/', '_').replace('\\', '_')
                val file = File(dir, safe)
                app.contentResolver.openInputStream(uri)?.use { input ->
                    file.outputStream().use { output -> input.copyTo(output) }
                } ?: error("无法读取所选文件")

                // 系统文件选择器会让应用短暂进入后台，Android 可能冻结进程并断开 TCP。
                // 回到前台后先等节点恢复连接，再发送请求。
                _state.update { it.copy(info = "正在准备 $name") }
                delay(2000)

                val command = JSONObject().apply {
                    put("cmd", "send_file_many")
                    put("targets", JSONArray(targets.map { it.pubkey }))
                    put("path", file.absolutePath)
                }
                val result = NativeBridge.sendCommand(command.toString())
                _state.update {
                    it.copy(
                        info = if (result.contains("\"ok\":false")) {
                            "发送失败"
                        } else {
                            "正在发送 $name 给 ${targets.size} 人"
                        },
                    )
                }
            } catch (err: Exception) {
                _state.update { it.copy(error = "发送文件失败: ${err.message}") }
            }
        }
    }

    fun acceptOffer(offer: FileOffer) {
        val command = JSONObject().put("cmd", "accept").put("xfer", offer.xfer)
        NativeBridge.sendCommand(command.toString())
    }

    fun rejectOffer(offer: FileOffer) {
        val command = JSONObject().put("cmd", "reject").put("xfer", offer.xfer)
        NativeBridge.sendCommand(command.toString())
        _state.update { it.copy(offers = it.offers.filterNot { o -> o.xfer == offer.xfer }) }
    }

    fun stop() {
        val app = getApplication<Application>()
        app.stopService(Intent(app, AsaService::class.java))
        runCatching { NativeBridge.stop() }
        val groups = _state.value.groups
        _state.update {
            UiState(
                screen = if (groups.isEmpty()) Screen.Connect else Screen.Groups,
                nick = it.nick,
                groups = groups,
                error = null,
                info = "已退出频道",
            )
        }
        reloadGroups(app)
    }

    /// 回到频道列表（主页），但保持当前节点和后台服务运行。
    fun backToGroups() {
        _state.update { it.copy(screen = Screen.Groups) }
        reloadGroups(getApplication())
    }

    fun renameGroup(roomId: String, name: String) {
        val app = getApplication<Application>()
        val trimmed = name.trim()
        if (trimmed.isEmpty()) return
        runCatching {
            JSONObject(NativeBridge.renameGroup(dataDir(app), roomId, trimmed))
        }
        if (_state.value.roomId == roomId) {
            _state.update { it.copy(roomName = trimmed) }
        }
        reloadGroups(app)
    }

    fun clearError() = _state.update { it.copy(error = null) }
    fun clearInfo() = _state.update { it.copy(info = null) }
    fun clearMentioned() = _state.update { it.copy(mentioned = null) }

    override fun onCleared() {
        // 后台服务保持节点运行；只有用户确认退出频道时才调用 stop()。
        pollJob.cancel()
        super.onCleared()
    }

    private fun preview(text: String): String {
        val flat = text.replace('\n', ' ').trim()
        return if (flat.length <= 60) flat else flat.take(59) + "…"
    }

    private fun displayName(uri: Uri): String? {
        val app = getApplication<Application>()
        return app.contentResolver.query(uri, null, null, null, null)?.use { cursor ->
            val index = cursor.getColumnIndex(OpenableColumns.DISPLAY_NAME)
            if (index >= 0 && cursor.moveToFirst()) cursor.getString(index) else null
        } ?: uri.lastPathSegment
    }

    private fun handleEvents(json: String) {
        val app = getApplication<Application>()
        val events = JSONArray(json)
        var current = _state.value
        val messages = current.messages.toMutableList()
        var members = current.members
        var host = current.host
        var me = current.me
        var nickname = current.nick
        var roomName = current.roomName
        val offers = current.offers.toMutableList()
        val transfers = current.transfers.associateBy { it.xfer }.toMutableMap()
        var info = current.info
        var error = current.error
        var mentioned = current.mentioned
        var screen = current.screen

        for (index in 0 until events.length()) {
            val event = events.getJSONObject(index)
            when (event.optString("type")) {
                "chat" -> {
                    val text = event.optString("text")
                    val msgId = event.optString("id")
                    val fromNick = event.optString("nick")
                    val mine = event.optBoolean("mine")
                    val mentions = event.optJSONArray("mentions")?.toStringList().orEmpty()
                    val mentionedMe = !mine && me.isNotBlank() && mentions.contains(me)
                    if (mentionedMe) mentioned = fromNick
                    if (!mine && !AppVisibility.foreground) {
                        NotificationHelper.notifyMessage(app, fromNick, text, mentionedMe)
                    }
                    val reply = event.optJSONObject("reply")?.let { ref ->
                        ReplyRef(
                            id = ref.optString("id"),
                            nick = ref.optString("nick"),
                            preview = ref.optString("preview"),
                        )
                    }
                    messages += ChatMessage(
                        key = nextKey++,
                        id = msgId,
                        nick = fromNick,
                        text = text,
                        ts = event.optLong("ts"),
                        mine = mine,
                        system = false,
                        mentionsMe = mentionedMe,
                        reply = reply,
                    )
                }
                "system" -> messages += ChatMessage(
                    key = nextKey++,
                    id = "",
                    nick = "系统",
                    text = event.optString("text"),
                    ts = event.optLong("ts"),
                    mine = false,
                    system = true,
                    mentionsMe = false,
                    reply = null,
                )
                "members" -> {
                    val list = event.optJSONArray("members")
                    val parsed = mutableListOf<Member>()
                    if (list != null) {
                        for (i in 0 until list.length()) {
                            val item = list.getJSONObject(i)
                            parsed += Member(
                                pubkey = item.optString("pubkey"),
                                peerId = item.optString("peer_id"),
                                nick = item.optString("nick"),
                                addr = item.optString("addr").ifBlank { null },
                                host = item.optBoolean("host"),
                                me = item.optBoolean("me"),
                            )
                        }
                    }
                    members = parsed
                    host = event.optString("host").ifBlank { null }
                    me = event.optString("me", me)
                    parsed.firstOrNull { it.me }?.let { nickname = it.nick }
                }
                "file_offer" -> offers += FileOffer(
                    xfer = event.optString("xfer"),
                    fromNick = event.optString("from_nick"),
                    name = event.optString("name"),
                    size = event.optLong("size"),
                )
                "file_progress" -> {
                    val xfer = event.optString("xfer")
                    transfers[xfer] = Transfer(
                        xfer = xfer,
                        name = event.optString("name"),
                        done = event.optLong("done"),
                        total = event.optLong("total"),
                        outgoing = event.optBoolean("outgoing"),
                    )
                }
                "file_complete" -> {
                    val xfer = event.optString("xfer")
                    transfers.remove(xfer)
                    offers.removeAll { it.xfer == xfer }
                    val name = event.optString("name")
                    if (event.optBoolean("ok")) {
                        info = if (event.optBoolean("outgoing")) "已发送 $name" else "已接收 $name"
                    } else {
                        error = "文件传输失败: $name · ${event.optString("msg")}"
                    }
                }
                "room_name" -> roomName = event.optString("name")
                "info" -> info = event.optString("text")
                "shutdown" -> {
                    screen = Screen.Connect
                    info = "节点已停止"
                }
            }
        }

        if (messages.size > 4000) {
            messages.subList(0, 1000).clear()
        }

        _state.value = current.copy(
            screen = screen,
            messages = messages,
            members = members,
            host = host,
            me = me,
            nick = nickname,
            roomName = roomName,
            offers = offers,
            transfers = transfers.values.sortedBy { it.name },
            info = info,
            error = error,
            mentioned = mentioned,
        )
    }
}

private fun JSONArray.toStringList(): List<String> {
    val list = ArrayList<String>(length())
    for (i in 0 until length()) list += optString(i)
    return list
}

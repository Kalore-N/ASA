package com.asa.chat

import android.Manifest
import android.app.Application
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.provider.OpenableColumns
import androidx.core.content.ContextCompat
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeoutOrNull
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
                        easytierEnabled = obj.optBoolean("easytier_enabled"),
                    )
                }
            }
        } else {
            reloadGroups(app, updateScreen = true)
        }
    }

    private fun dataDir(app: Application): String = File(app.filesDir, "asa").absolutePath

    private fun JSONArray?.toStringList(): List<String> {
        if (this == null) return emptyList()
        return buildList {
            for (index in 0 until length()) {
                val value = optString(index).trim()
                if (value.isNotEmpty()) add(value)
            }
        }
    }

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
                easytierEnabled = item.optBoolean("easytier_enabled"),
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

    fun discoverLan() {
        if (_state.value.lanLoading) return
        _state.update { it.copy(lanLoading = true) }
        viewModelScope.launch(Dispatchers.IO) {
            val app = getApplication<Application>()
            val lock = app.getSystemService(android.net.wifi.WifiManager::class.java)
                ?.createMulticastLock("asa-lan")
                ?.apply {
                    setReferenceCounted(false)
                    runCatching { acquire() }
                }
            val json = try {
                runCatching { NativeBridge.discoverChannels(2500) }.getOrNull()
            } finally {
                runCatching { lock?.release() }
            }
            android.util.Log.i("AsaLan", "discover: $json")
            val channels = runCatching {
                val obj = JSONObject(json ?: "{}")
                if (!obj.optBoolean("ok")) return@runCatching emptyList()
                val array = obj.optJSONArray("channels") ?: return@runCatching emptyList()
                buildList {
                    for (index in 0 until array.length()) {
                        val item = array.optJSONObject(index) ?: continue
                        val room = item.optString("room")
                        if (room.isBlank()) continue
                        add(
                            LanChannel(
                                room = room,
                                name = item.optString("name").ifBlank {
                                    "频道 ${room.take(8)}"
                                },
                                host = item.optString("host"),
                                addr = item.optString("addr"),
                                joinAddr = item.optString("join_addr"),
                                open = item.optBoolean("open"),
                            ),
                        )
                    }
                }
            }.getOrDefault(emptyList())
            _state.update { it.copy(lanChannels = channels, lanLoading = false) }
        }
    }

    fun joinLan(channel: LanChannel, password: String) {
        viewModelScope.launch(Dispatchers.IO) {
            val app = getApplication<Application>()
            val result = runCatching {
                JSONObject(
                    NativeBridge.joinDiscoveredChannel(
                        dataDir(app),
                        channel.joinAddr,
                        channel.room,
                        password,
                    ),
                )
            }.getOrNull()
            android.util.Log.i("AsaLan", "join: channel=${channel.name} result=$result")
            if (result?.optBoolean("ok") == true) {
                val roomId = result.optString("room_id")
                reloadGroups(app)
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
            } else {
                _state.update {
                    it.copy(error = result?.optString("error") ?: "加入局域网频道失败")
                }
            }
        }
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

    fun toggleEasytier(roomId: String, enabled: Boolean) {
        if (_state.value.connecting) return
        viewModelScope.launch(Dispatchers.IO) {
            val app = getApplication<Application>()
            val result = runCatching {
                JSONObject(NativeBridge.setEasytierEnabled(dataDir(app), roomId, enabled))
            }.getOrNull()
            if (result?.optBoolean("ok") != true) {
                _state.update {
                    it.copy(error = result?.optString("error") ?: "保存 EasyTier 开关失败")
                }
                return@launch
            }
            if (!enabled && EasyTierController.activeRoom == roomId) {
                EasyTierController.stop(app)
                _state.update {
                    it.copy(
                        easytierEnabled = false,
                        easytierIp = null,
                        easytierError = null,
                        info = "EasyTier 已关闭",
                    )
                }
            }
            reloadGroups(app)
            // 进程重启后 UiState.roomId 可能为空，用 native 状态补一次，
            // 否则在频道列表关闭当前频道的 ET 只会改配置，VPN 不会停。
            val runningRoom = if (runCatching { NativeBridge.isRunning() }.getOrDefault(false)) {
                runCatching {
                    JSONObject(NativeBridge.status()).optString("room_id")
                }.getOrDefault("")
            } else {
                ""
            }
            val currentRoom = _state.value.roomId.ifBlank { runningRoom }
            if (!enabled && currentRoom == roomId) {
                EasyTierController.stop(app)
            }
            // 当前频道需要重启才能应用虚拟 IP / 关闭 ET。
            if (currentRoom == roomId) {
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
        }
    }

    fun start(
        mode: String,
        nick: String,
        invite: String?,
        portText: String,
        connectText: String,
        appConfig: AppConfig,
        roomId: String? = null,
        joinPassword: String = "",
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
                if (joinPassword.isNotBlank()) put("join_password", joinPassword)
                put("connect", JSONArray(connects))
            }.toString()

            var obj = runCatching { JSONObject(NativeBridge.start(config)) }.getOrElse { error ->
                JSONObject()
                    .put("ok", false)
                    .put("error", error.message ?: "启动失败")
            }
            if (obj.optBoolean("ok")) {
                val etEnabled = obj.optBoolean("easytier_enabled")
                val etNodes = obj.optJSONArray("et_nodes").toStringList()
                val etNetwork = obj.optString("et_network")
                val etSecret = obj.optString("et_secret")
                val etIpv4 = obj.optString("et_ipv4")
                var etError: String? = null
                android.util.Log.i(
                    "AsaEasyTier",
                    "native start: enabled=$etEnabled nodes=$etNodes network=${etNetwork.take(10)} ip=$etIpv4",
                )

                if (!etEnabled) {
                    // 切到未开启 EasyTier 的频道时，清掉上一个频道的 ET/VPN。
                    EasyTierController.stop(app)
                } else if (etNodes.isNotEmpty() && etNetwork.isNotBlank() &&
                    etSecret.isNotBlank() && etIpv4.isNotBlank()
                ) {
                    if (!EasyTierController.ensureVpnPermission(app)) {
                        etError = EasyTierController.lastError ?: "未获得 VPN 授权"
                        EasyTierController.stop(app)
                        android.util.Log.w("AsaEasyTier", "vpn permission failed: $etError")
                    } else if (!EasyTierController.start(
                            app,
                            etNetwork,
                            etSecret,
                            "$etIpv4/16",
                            etNodes,
                            obj.optString("room_id"),
                        )
                    ) {
                        etError = EasyTierController.lastError ?: "EasyTier 启动失败"
                        EasyTierController.stop(app)
                        android.util.Log.w("AsaEasyTier", "controller start failed: $etError")
                    } else {
                        // 等 TUN 建立，再用虚拟 IP 重启 ASA 节点。
                        Thread.sleep(1500)
                        val restart = JSONObject(config).apply {
                            put("mode", "chat")
                            put("room_id", obj.optString("room_id"))
                            put("advertise", "$etIpv4:$port")
                            put("et_nodes", JSONArray(etNodes))
                            remove("invite")
                        }
                        val second = runCatching {
                            JSONObject(NativeBridge.start(restart.toString()))
                        }.getOrNull()
                        if (second?.optBoolean("ok") == true) {
                            obj = second
                        } else {
                            etError = second?.optString("error")?.ifBlank { null }
                                ?: "使用 EasyTier 地址重启频道失败"
                            EasyTierController.stop(app)
                            // 第二遍启动失败时，恢复第一遍的局域网节点。
                            val fallback = runCatching {
                                JSONObject(NativeBridge.start(config))
                            }.getOrNull()
                            if (fallback?.optBoolean("ok") == true) {
                                obj = fallback
                            }
                        }
                    }
                } else if (etEnabled) {
                    etError = "此频道已开启 EasyTier，但没有可用节点"
                    EasyTierController.stop(app)
                    android.util.Log.w("AsaEasyTier", etError)
                }

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
                        easytierEnabled = etEnabled,
                        easytierIp = if (etError == null && etEnabled) etIpv4 else null,
                        easytierError = etError,
                    )
                }
            } else {
                _state.update {
                    it.copy(
                        connecting = false,
                        error = obj.optString("error").ifBlank { "启动失败" },
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

    fun sync() {
        viewModelScope.launch(Dispatchers.IO) {
            val command = JSONObject().put("cmd", "sync").toString()
            val result = runCatching { NativeBridge.sendCommand(command) }.getOrNull()
            if (result == null || result.contains("\"ok\":false")) {
                _state.update { it.copy(error = "同步请求失败") }
            }
        }
    }

    fun openDownloads() {
        val app = getApplication<Application>()
        val dir = File(
            _state.value.downloads.ifBlank {
                File(app.filesDir, "asa/runtime/downloads").absolutePath
            },
        )
        dir.mkdirs()
        val uri = runCatching {
            androidx.core.content.FileProvider.getUriForFile(
                app,
                "${app.packageName}.fileprovider",
                dir,
            )
        }.getOrNull()
        if (uri != null) {
            val intent = Intent(Intent.ACTION_VIEW).apply {
                setDataAndType(uri, "vnd.android.document/directory")
                addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_ACTIVITY_NEW_TASK)
            }
            val chooser = Intent.createChooser(intent, "打开下载目录")
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
            if (runCatching { app.startActivity(chooser) }.isSuccess) return
        }
        val clipboard = app.getSystemService(android.content.ClipboardManager::class.java)
        clipboard?.setPrimaryClip(
            android.content.ClipData.newPlainText("ASA 下载目录", dir.absolutePath),
        )
        _state.update {
            it.copy(info = "下载目录: ${dir.absolutePath}（路径已复制）")
        }
    }

    private suspend fun ensureMic(): Boolean {
        val app = getApplication<Application>()
        if (ContextCompat.checkSelfPermission(app, Manifest.permission.RECORD_AUDIO) ==
            PackageManager.PERMISSION_GRANTED
        ) {
            return true
        }
        val deferred = CompletableDeferred<Boolean>()
        withContext(Dispatchers.Main) {
            MicPermission.request(app) { granted -> deferred.complete(granted) }
        }
        return withTimeoutOrNull(60_000) { deferred.await() } ?: false
    }

    private fun sendCallCommand(command: JSONObject): Boolean {
        val result = runCatching { NativeBridge.sendCommand(command.toString()) }.getOrNull()
        if (result == null || result.contains("\"ok\":false")) {
            _state.update { it.copy(error = "语音命令发送失败") }
            return false
        }
        return true
    }

    fun startCall(target: String) {
        if (_state.value.voiceRecording) {
            _state.update { it.copy(error = "录音中不能发起通话") }
            return
        }
        viewModelScope.launch(Dispatchers.IO) {
            if (!ensureMic()) {
                _state.update { it.copy(error = "未获得麦克风权限") }
                return@launch
            }
            val nick = _state.value.members.firstOrNull { it.pubkey == target }?.nick.orEmpty()
            if (sendCallCommand(JSONObject().put("cmd", "call").put("target", target))) {
                _state.update {
                    it.copy(call = CallState(CallStatus.Calling, peer = target, nick = nick))
                }
            }
        }
    }

    fun acceptCall() {
        if (_state.value.voiceRecording) {
            _state.update { it.copy(error = "录音中不能接听通话") }
            return
        }
        val callId = _state.value.call.callId
        if (callId.isBlank()) return
        viewModelScope.launch(Dispatchers.IO) {
            if (!ensureMic()) {
                _state.update { it.copy(error = "未获得麦克风权限") }
                return@launch
            }
            sendCallCommand(JSONObject().put("cmd", "accept_call").put("call_id", callId))
        }
    }

    fun rejectCall() {
        val callId = _state.value.call.callId
        runCatching {
            NativeBridge.sendCommand(
                JSONObject().put("cmd", "reject_call").put("call_id", callId).toString(),
            )
        }
        clearCall()
    }

    fun hangupCall() {
        runCatching {
            NativeBridge.sendCommand(JSONObject().put("cmd", "hangup_call").toString())
        }
        clearCall()
        _state.update { it.copy(info = "已挂断") }
    }

    fun toggleCallMute() {
        _state.update {
            val muted = !it.call.muted
            it.copy(call = it.call.copy(muted = muted))
        }
    }

    fun toggleSpeaker() {
        val app = getApplication<Application>()
        _state.update {
            val speaker = !it.call.speaker
            VoiceCallManager.setSpeaker(app, speaker)
            it.copy(call = it.call.copy(speaker = speaker))
        }
    }

    private fun startCallAudio() {
        val app = getApplication<Application>()
        runCatching {
            VoiceCallManager.start(app) { frame ->
                if (!_state.value.call.muted && _state.value.call.status == CallStatus.Active) {
                    sendAudioFrame(frame)
                }
            }
            VoiceCallManager.setSpeaker(app, _state.value.call.speaker)
            ContextCompat.startForegroundService(app, Intent(app, AsaCallService::class.java))
        }.onFailure { error ->
            _state.update { it.copy(error = "麦克风启动失败: ${error.message}") }
            hangupCall()
        }
    }

    private fun sendAudioFrame(frame: ByteArray) {
        val data = android.util.Base64.encodeToString(frame, android.util.Base64.NO_WRAP)
        val command = JSONObject().put("cmd", "audio_frame").put("data", data).toString()
        runCatching { NativeBridge.sendCommand(command) }
    }

    private fun clearCall() {
        val app = getApplication<Application>()
        VoiceCallManager.stop(app)
        VoiceCallManager.stopMessage()
        app.stopService(Intent(app, AsaCallService::class.java))
        _state.update { it.copy(call = CallState(), voiceRecording = false) }
    }

    fun startVoiceMessage(target: String) {
        if (_state.value.call.status != CallStatus.Idle) {
            _state.update { it.copy(error = "通话中不能录制语音消息") }
            return
        }
        viewModelScope.launch(Dispatchers.IO) {
            if (!ensureMic()) {
                _state.update { it.copy(error = "未获得麦克风权限") }
                return@launch
            }
            val app = getApplication<Application>()
            val appConfig = ConfigStore.load(app)
            val base = when {
                appConfig.voiceRecordDir.isNotBlank() -> File(appConfig.voiceRecordDir)
                _state.value.downloads.isNotBlank() -> File(_state.value.downloads)
                else -> File(app.filesDir, "asa/runtime/downloads")
            }
            val dir = File(base, "voice")
            if (!runCatching { dir.mkdirs() || dir.isDirectory }.getOrDefault(false)) {
                _state.update { it.copy(error = "语音目录不可写: ${dir.absolutePath}") }
                return@launch
            }
            val path = File(dir, "voice-${System.currentTimeMillis()}.wav")
            runCatching { VoiceCallManager.startMessage(app, path) }
                .onSuccess {
                    _state.update {
                        it.copy(
                            voiceRecording = true,
                            voiceTarget = target,
                            info = "录音中… 再次点击麦克风停止并发送",
                        )
                    }
                }
                .onFailure { error ->
                    _state.update { it.copy(error = "录音失败: ${error.message}") }
                }
        }
    }

    fun stopVoiceMessage() {
        val path = VoiceCallManager.stopMessage()
        val target = _state.value.voiceTarget
        _state.update { it.copy(voiceRecording = false, voiceTarget = "") }
        if (path.isNullOrBlank()) {
            _state.update { it.copy(error = "没有录到音频") }
            return
        }
        val file = File(path)
        if (!file.exists() || file.length() <= 44) {
            _state.update { it.copy(error = "录音为空") }
            return
        }
        val command = JSONObject()
            .put("cmd", "send_file")
            .put("target", target)
            .put("path", path)
        if (runCatching { NativeBridge.sendCommand(command.toString()) }.isSuccess) {
            _state.update { it.copy(info = "语音消息已发送") }
        } else {
            _state.update { it.copy(error = "发送语音消息失败") }
        }
    }

    fun playLastVoice() {
        val path = _state.value.lastVoicePath
        if (path.isNullOrBlank()) {
            _state.update { it.copy(error = "还没有收到语音消息") }
            return
        }
        if (!VoiceCallManager.playFile(path)) {
            _state.update { it.copy(error = "播放语音失败") }
        }
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
        EasyTierController.stop(app)
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
        var lastVoicePath = current.lastVoicePath

        for (index in 0 until events.length()) {
            val event = events.getJSONObject(index)
            when (event.optString("type")) {
                "chat" -> {
                    val text = event.optString("text")
                    val msgId = event.optString("id")
                    if (msgId.isNotBlank() && messages.any { it.id == msgId }) {
                        continue
                    }
                    val fromNick = event.optString("nick")
                    val mine = event.optBoolean("mine")
                    val mentions = event.optJSONArray("mentions")?.toStringList().orEmpty()
                    val history = event.optBoolean("history")
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
                    val chat = ChatMessage(
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
                    if (history) {
                        val position = messages.indexOfFirst { it.ts > chat.ts }
                        if (position < 0) messages.add(chat) else messages.add(position, chat)
                    } else {
                        messages.add(chat)
                    }
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
                "call_ringing" -> {
                    _state.update {
                        it.copy(
                            call = CallState(
                                CallStatus.Calling,
                                peer = event.optString("peer"),
                                nick = event.optString("nick"),
                                callId = event.optString("call_id"),
                            ),
                        )
                    }
                }
                "incoming_call" -> {
                    _state.update {
                        it.copy(
                            call = CallState(
                                CallStatus.Incoming,
                                peer = event.optString("peer"),
                                nick = event.optString("nick"),
                                callId = event.optString("call_id"),
                            ),
                        )
                    }
                }
                "call_accepted" -> {
                    _state.update {
                        it.copy(
                            call = it.call.copy(
                                status = CallStatus.Active,
                                peer = event.optString("peer"),
                                nick = event.optString("nick"),
                                callId = event.optString("call_id"),
                            ),
                        )
                    }
                    startCallAudio()
                }
                "call_ended" -> {
                    val reason = event.optString("reason")
                    clearCall()
                    info = "通话结束: $reason"
                }
                "audio_frame" -> {
                    runCatching {
                        val data = android.util.Base64.decode(
                            event.optString("data"),
                            android.util.Base64.NO_WRAP,
                        )
                        VoiceCallManager.playFrame(data)
                    }
                }
                "file_complete" -> {
                    val xfer = event.optString("xfer")
                    transfers.remove(xfer)
                    offers.removeAll { it.xfer == xfer }
                    val name = event.optString("name")
                    if (event.optBoolean("ok")) {
                        info = if (event.optBoolean("outgoing")) "已发送 $name" else "已接收 $name"
                        val path = event.optString("path")
                        if (!event.optBoolean("outgoing") && path.isNotBlank() &&
                            name.endsWith(".wav", true)
                        ) {
                            lastVoicePath = path
                            info = "收到语音消息；更多 → 播放最近语音"
                        }
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
            call = _state.value.call,
            voiceRecording = _state.value.voiceRecording,
            voiceTarget = _state.value.voiceTarget,
            lastVoicePath = lastVoicePath,
        )
    }
}

private fun JSONArray.toStringList(): List<String> {
    val list = ArrayList<String>(length())
    for (i in 0 until length()) list += optString(i)
    return list
}

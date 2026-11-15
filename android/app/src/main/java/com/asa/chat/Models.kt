package com.asa.chat

data class ReplyRef(
    val id: String,
    val nick: String,
    val preview: String,
)

data class ChatMessage(
    val key: Long,
    val id: String,
    val nick: String,
    val text: String,
    val ts: Long,
    val mine: Boolean,
    val system: Boolean,
    val mentionsMe: Boolean,
    val reply: ReplyRef?,
    /** 语音消息的本地 WAV 路径；非空时聊天气泡显示成可点击语音条。 */
    val voicePath: String? = null,
)

data class Member(
    val pubkey: String,
    val peerId: String,
    val nick: String,
    val addr: String?,
    val host: Boolean,
    val me: Boolean,
)

data class FileOffer(
    val xfer: String,
    val fromNick: String,
    val name: String,
    val size: Long,
)

data class Transfer(
    val xfer: String,
    val name: String,
    val done: Long,
    val total: Long,
    val outgoing: Boolean,
)

data class LanChannel(
    val room: String,
    val name: String,
    val host: String,
    val addr: String,
    val joinAddr: String,
    val open: Boolean,
)

data class SavedGroup(
    val id: String,
    val name: String,
    val lastUsed: Long,
    val active: Boolean,
    val easytierEnabled: Boolean = false,
)

enum class Screen { Connect, Groups, Chat }

enum class CallStatus { Idle, Calling, Incoming, Active }

data class CallState(
    val status: CallStatus = CallStatus.Idle,
    val peer: String = "",
    val nick: String = "",
    val callId: String = "",
    val muted: Boolean = false,
    val speaker: Boolean = false,
)

data class VoiceRoomMember(
    val pubkey: String,
    val nick: String,
)

data class VoiceRoomState(
    val joined: Boolean = false,
    val members: List<VoiceRoomMember> = emptyList(),
    val muted: Boolean = false,
    val speaker: Boolean = false,
)

data class UiState(
    val screen: Screen = Screen.Connect,
    val connecting: Boolean = false,
    val error: String? = null,
    val info: String? = null,
    val nick: String = "",
    val me: String = "",
    val roomId: String = "",
    val roomName: String = "",
    val groups: List<SavedGroup> = emptyList(),
    val invite: String = "",
    val listen: String = "",
    val downloads: String = "",
    val host: String? = null,
    val members: List<Member> = emptyList(),
    val messages: List<ChatMessage> = emptyList(),
    val offers: List<FileOffer> = emptyList(),
    val transfers: List<Transfer> = emptyList(),
    val mentioned: String? = null,
    val replyTarget: ChatMessage? = null,
    val easytierEnabled: Boolean = false,
    val easytierIp: String? = null,
    val easytierError: String? = null,
    val lanChannels: List<LanChannel> = emptyList(),
    val lanLoading: Boolean = false,
    val call: CallState = CallState(),
    val voiceRecording: Boolean = false,
    val voiceTarget: String = "",
    val lastVoicePath: String? = null,
    val voiceRoom: VoiceRoomState = VoiceRoomState(),
    /** 录好但还没选接收人的语音消息。 */
    val pendingVoicePath: String? = null,
    /** 正在播放的语音条路径。 */
    val playingVoice: String? = null,
) {
    val hostNick: String
        get() = members.firstOrNull { it.pubkey == host }?.nick ?: "—"

    val online: Int
        get() = members.size.coerceAtLeast(1)

    val others: List<Member>
        get() = members.filter { !it.me }

    fun isMe(member: Member): Boolean = member.me || member.pubkey == me
}

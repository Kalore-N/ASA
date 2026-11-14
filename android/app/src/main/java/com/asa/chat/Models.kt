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

enum class Screen { Connect, Chat }

data class UiState(
    val screen: Screen = Screen.Connect,
    val connecting: Boolean = false,
    val error: String? = null,
    val info: String? = null,
    val nick: String = "",
    val me: String = "",
    val roomId: String = "",
    val roomName: String = "",
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
) {
    val hostNick: String
        get() = members.firstOrNull { it.pubkey == host }?.nick ?: "—"

    val online: Int
        get() = members.size.coerceAtLeast(1)

    val others: List<Member>
        get() = members.filter { !it.me }

    fun isMe(member: Member): Boolean = member.me || member.pubkey == me
}

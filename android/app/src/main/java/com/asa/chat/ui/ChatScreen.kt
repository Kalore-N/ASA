package com.asa.chat.ui

import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Call
import androidx.compose.material.icons.filled.CallEnd
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.Link
import androidx.compose.material.icons.filled.Groups
import androidx.compose.material.icons.filled.Mic
import androidx.compose.material.icons.filled.MicOff
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material.icons.filled.PlayArrow
import androidx.compose.material.icons.filled.Stop
import androidx.compose.material.icons.filled.VolumeOff
import androidx.compose.material.icons.filled.VolumeUp
import androidx.compose.material.icons.filled.People
import androidx.compose.material.icons.filled.Send
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Checkbox
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.input.pointer.PointerEventPass
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.asa.chat.AsaViewModel
import com.asa.chat.R
import com.asa.chat.ChatMessage
import com.asa.chat.FileOffer
import com.asa.chat.Member
import com.asa.chat.CallStatus
import com.asa.chat.Transfer
import com.asa.chat.UiState
import java.time.Instant
import java.time.ZoneId
import java.time.format.DateTimeFormatter

private val BubbleShape = RoundedCornerShape(8.dp)
private val FieldShape = RoundedCornerShape(6.dp)
private val SheetShape = RoundedCornerShape(topStart = 8.dp, topEnd = 8.dp)
private val DialogShape = RoundedCornerShape(8.dp)
private val TimeFormat = DateTimeFormatter.ofPattern("HH:mm")

@OptIn(ExperimentalMaterial3Api::class, ExperimentalFoundationApi::class)
@Composable
fun ChatScreen(state: UiState, vm: AsaViewModel, onOpenSettings: () -> Unit) {
    var input by rememberSaveable(stateSaver = TextFieldValue.Saver) {
        mutableStateOf(TextFieldValue(""))
    }
    var showMembers by remember { mutableStateOf(false) }
    var showInvite by remember { mutableStateOf(false) }
    var showMenu by remember { mutableStateOf(false) }
    var showTransfers by remember { mutableStateOf(false) }
    var showCallPicker by remember { mutableStateOf(false) }
    var showVoicePicker by remember { mutableStateOf(false) }
    var showVoiceRoom by remember { mutableStateOf(false) }
    var showNick by remember { mutableStateOf(false) }
    var showRename by remember { mutableStateOf(false) }
    var showConnect by remember { mutableStateOf(false) }
    var showExitConfirm by remember { mutableStateOf(false) }
    var showTargets by remember { mutableStateOf(false) }
    var pendingTargets by remember { mutableStateOf<List<Member>>(emptyList()) }
    var selectedTargets by remember { mutableStateOf<Set<String>>(emptySet()) }
    val snackbar = remember { SnackbarHostState() }
    val listState = rememberLazyListState()
    val clipboard = LocalClipboardManager.current

    val pickFile = rememberLauncherForActivityResult(ActivityResultContracts.OpenDocument()) { uri ->
        if (uri != null && pendingTargets.isNotEmpty()) {
            vm.sendFile(uri, pendingTargets)
        }
    }

    LaunchedEffect(state.messages.size) {
        if (state.messages.isNotEmpty()) {
            listState.animateScrollToItem(state.messages.lastIndex)
        }
    }
    LaunchedEffect(state.error) {
        state.error?.let {
            snackbar.showSnackbar(it)
            vm.clearError()
        }
    }
    LaunchedEffect(state.info) {
        state.info?.let {
            snackbar.showSnackbar(it)
            vm.clearInfo()
        }
    }
    LaunchedEffect(state.pendingVoicePath) {
        showVoicePicker = state.pendingVoicePath != null
    }
    LaunchedEffect(state.mentioned) {
        state.mentioned?.let {
            snackbar.showSnackbar("$it 提到了你")
            vm.clearMentioned()
        }
    }

    Scaffold(
        containerColor = MaterialTheme.colorScheme.background,
        snackbarHost = { SnackbarHost(snackbar) },
        topBar = {
            TopAppBar(
                colors = TopAppBarDefaults.topAppBarColors(
                    containerColor = MaterialTheme.colorScheme.surface,
                    titleContentColor = MaterialTheme.colorScheme.onSurface,
                ),
                title = {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Image(
                            painter = painterResource(R.drawable.asa_logo),
                            contentDescription = "ASA logo",
                            modifier = Modifier
                                .size(28.dp)
                                .clip(RoundedCornerShape(6.dp)),
                        )
                        Spacer(Modifier.size(8.dp))
                        Column {
                            Text(
                                state.roomName.ifBlank { "频道 ${state.roomId.take(8)}" },
                                fontSize = 16.sp,
                                fontWeight = FontWeight.SemiBold,
                            )
                            Text(
                                "频道 ${state.roomId.take(8)} · 在线 ${state.online} · 服主 ${state.hostNick}",
                                fontSize = 11.sp,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                    }
                },
                actions = {
                    IconButton(onClick = { showVoiceRoom = true }) {
                        Icon(
                            Icons.Filled.Groups,
                            contentDescription = "频道语音房",
                            tint = if (state.voiceRoom.joined) {
                                MaterialTheme.colorScheme.primary
                            } else {
                                MaterialTheme.colorScheme.onSurface
                            },
                        )
                    }
                    IconButton(onClick = { showCallPicker = true }) {
                        Icon(Icons.Filled.Call, contentDescription = "语音通话")
                    }
                    IconButton(onClick = { showMembers = true }) {
                        Icon(Icons.Filled.People, contentDescription = "成员")
                    }
                    IconButton(onClick = { showInvite = true }) {
                        Icon(Icons.Filled.Link, contentDescription = "邀请码")
                    }
                    Box {
                        IconButton(onClick = { showMenu = true }) {
                            Icon(Icons.Filled.MoreVert, contentDescription = "更多")
                        }
                        DropdownMenu(
                            expanded = showMenu,
                            onDismissRequest = { showMenu = false },
                        ) {
                            DropdownMenuItem(
                                text = { Text("同步聊天记录") },
                                onClick = {
                                    showMenu = false
                                    vm.sync()
                                },
                            )
                            DropdownMenuItem(
                                text = { Text("进行中的传输") },
                                onClick = {
                                    showMenu = false
                                    showTransfers = true
                                },
                            )
                            DropdownMenuItem(
                                text = { Text("发送语音消息") },
                                onClick = {
                                    showMenu = false
                                    showVoicePicker = true
                                },
                            )
                            if (state.lastVoicePath != null) {
                                DropdownMenuItem(
                                    text = { Text("播放最近语音") },
                                    onClick = {
                                        showMenu = false
                                        vm.playLastVoice()
                                    },
                                )
                            }
                            DropdownMenuItem(
                                text = { Text("打开下载目录") },
                                onClick = {
                                    showMenu = false
                                    vm.openDownloads()
                                },
                            )
                            DropdownMenuItem(
                                text = { Text("发送文件") },
                                onClick = {
                                    showMenu = false
                                    showTargets = true
                                },
                            )
                            DropdownMenuItem(
                                text = { Text("修改频道名称") },
                                onClick = {
                                    showMenu = false
                                    showRename = true
                                },
                            )
                            DropdownMenuItem(
                                text = { Text("修改昵称") },
                                onClick = {
                                    showMenu = false
                                    showNick = true
                                },
                            )
                            DropdownMenuItem(
                                text = { Text("手动连接") },
                                onClick = {
                                    showMenu = false
                                    showConnect = true
                                },
                            )
                            DropdownMenuItem(
                                text = { Text("切换频道") },
                                onClick = {
                                    showMenu = false
                                    vm.backToGroups()
                                },
                            )
                            DropdownMenuItem(
                                text = { Text("设置") },
                                onClick = {
                                    showMenu = false
                                    onOpenSettings()
                                },
                            )
                            HorizontalDivider()
                            DropdownMenuItem(
                                text = { Text("退出频道", color = MaterialTheme.colorScheme.error) },
                                onClick = {
                                    showMenu = false
                                    showExitConfirm = true
                                },
                            )
                        }
                    }
                },
            )
        },
        bottomBar = {
            Column {
                CallBar(state = state, vm = vm)
                Composer(
                    state = state,
                    input = input,
                    onInputChange = { input = it },
                    onSend = {
                        if (input.text.isNotBlank()) {
                            vm.send(input.text)
                            input = TextFieldValue("")
                        }
                    },
                    onCancelReply = vm::clearReply,
                    onAccept = vm::acceptOffer,
                    onReject = vm::rejectOffer,
                    onVoice = vm::toggleVoiceInput,
                )
            }
        },
    ) { padding ->
        LazyColumn(
            state = listState,
            modifier = Modifier
                .fillMaxWidth()
                .padding(padding),
            contentPadding = PaddingValues(horizontal = 12.dp, vertical = 10.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            items(state.messages, key = { it.key }) { message ->
                MessageRow(
                    message = message,
                    playingVoice = state.playingVoice,
                    onReply = { vm.setReply(message) },
                    onPlayVoice = vm::playVoice,
                )
            }
        }
    }

    if (showMembers) {
        ModalBottomSheet(
            onDismissRequest = { showMembers = false },
            containerColor = MaterialTheme.colorScheme.surface,
            shape = SheetShape,
        ) {
            Text(
                "在线成员 ${state.members.size}",
                modifier = Modifier.padding(horizontal = 18.dp, vertical = 8.dp),
                fontWeight = FontWeight.SemiBold,
            )
            LazyColumn(Modifier.padding(bottom = 24.dp)) {
                items(state.members, key = { it.pubkey }) { member ->
                    ListItem(
                        headlineContent = {
                            Text(
                                buildString {
                                    if (member.host) append("★ ")
                                    append(member.nick)
                                    if (member.me) append("（你）")
                                },
                            )
                        },
                        supportingContent = {
                            Text(
                                "${member.peerId.take(8)} · ${member.addr ?: "地址未知"}",
                                fontSize = 12.sp,
                            )
                        },
                    )
                }
            }
        }
    }

    if (showInvite) {
        AlertDialog(
            onDismissRequest = { showInvite = false },
            shape = DialogShape,
            title = { Text("邀请码") },
            text = {
                Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    Text(
                        "把这个邀请码发给朋友，对方在 ASA 里选择「加入群」即可。",
                        fontSize = 13.sp,
                    )
                    SelectionContainer {
                        Text(
                            state.invite,
                            color = MaterialTheme.colorScheme.primary,
                            fontSize = 12.sp,
                        )
                    }
                }
            },
            confirmButton = {
                TextButton(onClick = {
                    clipboard.setText(AnnotatedString(state.invite))
                    showInvite = false
                }) { Text("复制并关闭") }
            },
            dismissButton = {
                TextButton(onClick = { showInvite = false }) { Text("关闭") }
            },
        )
    }

    if (showTransfers) {
        AlertDialog(
            onDismissRequest = { showTransfers = false },
            shape = DialogShape,
            title = { Text("进行中的传输") },
            text = {
                if (state.transfers.isEmpty()) {
                    Text("暂无进行中的传输", fontSize = 13.sp)
                } else {
                    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                        state.transfers.forEach { TransferRow(it) }
                    }
                }
            },
            confirmButton = {
                TextButton(onClick = { showTransfers = false }) { Text("关闭") }
            },
        )
    }

    if (showCallPicker) {
        AlertDialog(
            onDismissRequest = { showCallPicker = false },
            shape = DialogShape,
            title = { Text("语音通话") },
            text = {
                val others = state.members.filter { !it.me }
                if (others.isEmpty()) {
                    Text("暂无其他在线成员", fontSize = 13.sp)
                } else {
                    LazyColumn(modifier = Modifier.heightIn(max = 360.dp)) {
                        items(others, key = { it.pubkey }) { member ->
                            TextButton(
                                onClick = {
                                    showCallPicker = false
                                    vm.startCall(member.pubkey)
                                },
                                modifier = Modifier.fillMaxWidth(),
                            ) {
                                Text("${member.nick} · ${member.peerId.take(8)}")
                            }
                        }
                    }
                }
            },
            confirmButton = {
                TextButton(onClick = { showCallPicker = false }) { Text("取消") }
            },
        )
    }

    if (showVoicePicker) {
        AlertDialog(
            onDismissRequest = {
                showVoicePicker = false
                vm.cancelPendingVoice()
            },
            shape = DialogShape,
            title = { Text(if (state.pendingVoicePath != null) "选择语音接收人" else "发送语音消息") },
            text = {
                val others = state.members.filter { !it.me }
                if (others.isEmpty()) {
                    Text("暂无其他在线成员", fontSize = 13.sp)
                } else {
                    LazyColumn(modifier = Modifier.heightIn(max = 360.dp)) {
                        items(others, key = { it.pubkey }) { member ->
                            TextButton(
                                onClick = {
                                    showVoicePicker = false
                                    if (state.pendingVoicePath != null) {
                                        vm.sendPendingVoice(member.pubkey)
                                    } else {
                                        vm.startVoiceMessage(member.pubkey)
                                    }
                                },
                                modifier = Modifier.fillMaxWidth(),
                            ) {
                                Text("${member.nick} · ${member.peerId.take(8)}")
                            }
                        }
                    }
                }
            },
            confirmButton = {
                TextButton(
                    onClick = {
                        showVoicePicker = false
                        vm.cancelPendingVoice()
                    },
                ) { Text("取消") }
            },
        )
    }

    if (showVoiceRoom) {
        val room = state.voiceRoom
        AlertDialog(
            onDismissRequest = { showVoiceRoom = false },
            shape = DialogShape,
            title = { Text("频道语音房") },
            text = {
                Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    if (room.joined) {
                        Text("已加入 · ${room.members.size} 人（含你）", fontSize = 13.sp)
                        room.members.forEach { member ->
                            Text(
                                "• ${member.nick}" + if (member.pubkey == state.me) "（你）" else "",
                                fontSize = 13.sp,
                            )
                        }
                        Row(verticalAlignment = Alignment.CenterVertically) {
                            TextButton(onClick = vm::toggleVoiceRoomMute) {
                                Text(if (room.muted) "取消静音" else "静音")
                            }
                            TextButton(onClick = vm::toggleVoiceRoomSpeaker) {
                                Text(if (room.speaker) "关闭免提" else "免提")
                            }
                        }
                        Text(
                            "语音房与 1:1 通话互斥；离开后可发起普通通话。",
                            fontSize = 11.sp,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    } else {
                        Text(
                            "加入后，频道内已加入语音房的成员可以互相实时通话（建议不超过 8 人）。",
                            fontSize = 13.sp,
                        )
                    }
                }
            },
            confirmButton = {
                if (room.joined) {
                    TextButton(onClick = {
                        showVoiceRoom = false
                        vm.leaveVoiceRoom()
                    }) { Text("退出语音房", color = MaterialTheme.colorScheme.error) }
                } else {
                    TextButton(onClick = {
                        showVoiceRoom = false
                        vm.joinVoiceRoom()
                    }) { Text("加入语音房") }
                }
            },
            dismissButton = {
                TextButton(onClick = { showVoiceRoom = false }) { Text("关闭") }
            },
        )
    }

    if (state.call.status == CallStatus.Incoming) {
        AlertDialog(
            onDismissRequest = {},
            shape = DialogShape,
            title = { Text("${state.call.nick} 来电") },
            text = { Text("是否接听语音通话？", fontSize = 13.sp) },
            confirmButton = {
                TextButton(onClick = { vm.acceptCall() }) { Text("接听") }
            },
            dismissButton = {
                TextButton(onClick = { vm.rejectCall() }) {
                    Text("拒绝", color = MaterialTheme.colorScheme.error)
                }
            },
        )
    }

    if (showNick) {
        var nick by remember { mutableStateOf(state.nick) }
        AlertDialog(
            onDismissRequest = { showNick = false },
            shape = DialogShape,
            title = { Text("修改昵称") },
            text = {
                OutlinedTextField(
                    value = nick,
                    onValueChange = { nick = it },
                    singleLine = true,
                    shape = FieldShape,
                    label = { Text("昵称") },
                )
            },
            confirmButton = {
                TextButton(onClick = {
                    vm.setNick(nick)
                    showNick = false
                }) { Text("保存") }
            },
            dismissButton = {
                TextButton(onClick = { showNick = false }) { Text("取消") }
            },
        )
    }

    if (showExitConfirm) {
        AlertDialog(
            onDismissRequest = { showExitConfirm = false },
            shape = DialogShape,
            title = { Text("退出频道？") },
            text = { Text("退出后将断开连接并停止后台运行；下次进入需要重新连接。") },
            confirmButton = {
                TextButton(onClick = {
                    showExitConfirm = false
                    vm.stop()
                }) {
                    Text("退出", color = MaterialTheme.colorScheme.error)
                }
            },
            dismissButton = {
                TextButton(onClick = { showExitConfirm = false }) { Text("取消") }
            },
        )
    }

    if (showRename) {
        var name by remember { mutableStateOf(state.roomName) }
        AlertDialog(
            onDismissRequest = { showRename = false },
            shape = DialogShape,
            title = { Text("修改频道名称（仅本机显示）") },
            text = {
                OutlinedTextField(
                    value = name,
                    onValueChange = { name = it },
                    singleLine = true,
                    shape = FieldShape,
                    label = { Text("频道名称") },
                    placeholder = { Text("频道 ${state.roomId.take(8)}") },
                )
            },
            confirmButton = {
                TextButton(onClick = {
                    vm.setRoomName(name)
                    showRename = false
                }) { Text("保存") }
            },
            dismissButton = {
                TextButton(onClick = { showRename = false }) { Text("取消") }
            },
        )
    }

    if (showConnect) {
        var addr by remember { mutableStateOf("") }
        AlertDialog(
            onDismissRequest = { showConnect = false },
            shape = DialogShape,
            title = { Text("手动连接") },
            text = {
                OutlinedTextField(
                    value = addr,
                    onValueChange = { addr = it },
                    singleLine = true,
                    shape = FieldShape,
                    label = { Text("ip:port") },
                    placeholder = { Text("192.168.1.8:7777") },
                )
            },
            confirmButton = {
                TextButton(onClick = {
                    vm.connect(addr)
                    showConnect = false
                }) { Text("连接") }
            },
            dismissButton = {
                TextButton(onClick = { showConnect = false }) { Text("取消") }
            },
        )
    }

    if (showTargets) {
        val targets = state.others
        AlertDialog(
            onDismissRequest = { showTargets = false },
            shape = DialogShape,
            title = { Text("选择文件接收人") },
            text = {
                if (targets.isEmpty()) {
                    Text("当前没有其他在线成员。", fontSize = 13.sp)
                } else {
                    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                        Row(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                            TextButton(
                                onClick = {
                                    selectedTargets = targets.map { it.pubkey }.toSet()
                                },
                                shape = FieldShape,
                            ) { Text("全选", fontSize = 12.sp) }
                            TextButton(
                                onClick = { selectedTargets = emptySet() },
                                shape = FieldShape,
                            ) { Text("清空", fontSize = 12.sp) }
                        }
                        LazyColumn(Modifier.heightIn(max = 300.dp)) {
                            items(targets, key = { it.pubkey }) { member ->
                                Row(
                                    modifier = Modifier
                                        .fillMaxWidth()
                                        .clickable {
                                            selectedTargets = if (member.pubkey in selectedTargets) {
                                                selectedTargets - member.pubkey
                                            } else {
                                                selectedTargets + member.pubkey
                                            }
                                        }
                                        .padding(vertical = 2.dp),
                                    verticalAlignment = Alignment.CenterVertically,
                                ) {
                                    Checkbox(
                                        checked = member.pubkey in selectedTargets,
                                        onCheckedChange = { checked ->
                                            selectedTargets = if (checked) {
                                                selectedTargets + member.pubkey
                                            } else {
                                                selectedTargets - member.pubkey
                                            }
                                        },
                                    )
                                    Text(
                                        "${member.nick}  ·  ${member.addr ?: ""}",
                                        fontSize = 13.sp,
                                        maxLines = 1,
                                        overflow = TextOverflow.Ellipsis,
                                    )
                                }
                            }
                        }
                    }
                }
            },
            confirmButton = {
                if (targets.isNotEmpty()) {
                    TextButton(
                        enabled = selectedTargets.isNotEmpty(),
                        onClick = {
                            pendingTargets = targets.filter { it.pubkey in selectedTargets }
                            showTargets = false
                            pickFile.launch(arrayOf("*/*"))
                        },
                    ) { Text("选择文件") }
                }
            },
            dismissButton = {
                TextButton(onClick = { showTargets = false }) { Text("取消") }
            },
        )
    }
}

@Composable
private fun CallBar(state: UiState, vm: AsaViewModel) {
    if (state.voiceRecording) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .background(MaterialTheme.colorScheme.errorContainer)
                .padding(horizontal = 12.dp, vertical = 8.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text("录音中…", color = MaterialTheme.colorScheme.onErrorContainer, fontSize = 12.sp)
            Spacer(Modifier.weight(1f))
            TextButton(onClick = vm::stopVoiceMessage) {
                Text(if (state.voiceTarget.isBlank()) "停止并选择接收人" else "停止并发送")
            }
        }
    }
    when (state.call.status) {
        CallStatus.Calling -> Row(
            modifier = Modifier
                .fillMaxWidth()
                .background(MaterialTheme.colorScheme.surfaceVariant)
                .padding(horizontal = 12.dp, vertical = 6.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text("正在呼叫 ${state.call.nick}…", fontSize = 12.sp)
            Spacer(Modifier.weight(1f))
            IconButton(onClick = vm::hangupCall) {
                Icon(Icons.Filled.CallEnd, contentDescription = "挂断")
            }
        }
        CallStatus.Active -> Row(
            modifier = Modifier
                .fillMaxWidth()
                .background(MaterialTheme.colorScheme.surfaceVariant)
                .padding(horizontal = 12.dp, vertical = 6.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(
                "通话中: ${state.call.nick}${if (state.call.muted) " (静音)" else ""}",
                fontSize = 12.sp,
                modifier = Modifier.weight(1f),
            )
            IconButton(onClick = vm::toggleCallMute) {
                Icon(
                    if (state.call.muted) Icons.Filled.VolumeOff else Icons.Filled.VolumeUp,
                    contentDescription = "静音",
                )
            }
            IconButton(onClick = vm::toggleSpeaker) {
                Icon(
                    if (state.call.speaker) Icons.Filled.VolumeUp else Icons.Filled.VolumeOff,
                    contentDescription = "免提",
                )
            }
            IconButton(onClick = vm::hangupCall) {
                Icon(Icons.Filled.CallEnd, contentDescription = "挂断")
            }
        }
        else -> Unit
    }
}

@Composable
private fun Composer(
    state: UiState,
    input: TextFieldValue,
    onInputChange: (TextFieldValue) -> Unit,
    onSend: () -> Unit,
    onCancelReply: () -> Unit,
    onAccept: (FileOffer) -> Unit,
    onReject: (FileOffer) -> Unit,
    onVoice: () -> Unit,
) {
    val text = input.text
    val mentionStart = text.lastIndexOf('@')
    val mentionQuery = if (mentionStart >= 0) text.substring(mentionStart + 1) else ""
    val mentionActive = mentionStart >= 0 && mentionQuery.none { it.isWhitespace() }
    val mentionCandidates = if (mentionActive) {
        buildList {
            state.members
                .filter { !it.me && it.nick.lowercase().startsWith(mentionQuery.lowercase()) }
                .forEach { add(it.nick) }
            if ("所有人".startsWith(mentionQuery)) add("所有人")
        }
    } else {
        emptyList()
    }

    Column(
        modifier = Modifier
            .fillMaxWidth()
            .background(MaterialTheme.colorScheme.surface)
            .navigationBarsPadding()
            .imePadding(),
    ) {
        state.offers.forEach { offer ->
            OfferRow(
                offer = offer,
                onAccept = { onAccept(offer) },
                onReject = { onReject(offer) },
            )
        }
        state.transfers.forEach { transfer ->
            TransferRow(transfer)
        }
        state.replyTarget?.let { target ->
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 10.dp, vertical = 6.dp)
                    .background(MaterialTheme.colorScheme.surfaceVariant, FieldShape)
                    .padding(horizontal = 10.dp, vertical = 6.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Column(Modifier.weight(1f)) {
                    Text(
                        "回复 ${target.nick}",
                        fontSize = 11.sp,
                        color = MaterialTheme.colorScheme.primary,
                    )
                    Text(
                        target.text.replace('\n', ' '),
                        fontSize = 12.sp,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                IconButton(onClick = onCancelReply, modifier = Modifier.size(30.dp)) {
                    Icon(Icons.Filled.Close, contentDescription = "取消回复", modifier = Modifier.size(16.dp))
                }
            }
        }
        if (mentionActive && mentionCandidates.isNotEmpty()) {
            LazyRow(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 8.dp, vertical = 4.dp),
                horizontalArrangement = Arrangement.spacedBy(6.dp),
            ) {
                items(mentionCandidates) { name ->
                    OutlinedButton(
                        onClick = {
                            val prefix = text.substring(0, mentionStart)
                            val next = "$prefix@$name "
                            onInputChange(TextFieldValue(next, selection = TextRange(next.length)))
                        },
                        shape = FieldShape,
                        contentPadding = PaddingValues(horizontal = 10.dp, vertical = 4.dp),
                    ) {
                        Text("@$name", fontSize = 12.sp)
                    }
                }
            }
        }
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 8.dp, vertical = 6.dp),
            verticalAlignment = Alignment.Bottom,
        ) {
            OutlinedTextField(
                value = input,
                onValueChange = onInputChange,
                modifier = Modifier.weight(1f),
                placeholder = { Text("输入消息，@ 提及成员") },
                shape = FieldShape,
                maxLines = 4,
            )
            Spacer(Modifier.size(4.dp))
            IconButton(
                onClick = onVoice,
                modifier = Modifier.size(52.dp),
            ) {
                Icon(
                    if (state.voiceRecording) Icons.Filled.MicOff else Icons.Filled.Mic,
                    contentDescription = if (state.voiceRecording) "停止录音" else "发送语音",
                    tint = if (state.voiceRecording) {
                        MaterialTheme.colorScheme.error
                    } else {
                        MaterialTheme.colorScheme.onSurfaceVariant
                    },
                )
            }
            Spacer(Modifier.size(2.dp))
            Button(
                onClick = onSend,
                shape = FieldShape,
                modifier = Modifier.height(52.dp),
                contentPadding = PaddingValues(horizontal = 14.dp),
                enabled = input.text.isNotBlank(),
            ) {
                Icon(Icons.Filled.Send, contentDescription = "发送", modifier = Modifier.size(18.dp))
            }
        }
    }
}

@Composable
private fun OfferRow(offer: FileOffer, onAccept: () -> Unit, onReject: () -> Unit) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = 10.dp, vertical = 4.dp)
            .border(1.dp, MaterialTheme.colorScheme.outline, FieldShape)
            .padding(horizontal = 10.dp, vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(Modifier.weight(1f)) {
            Text(
                "${offer.fromNick} 想发送文件",
                fontSize = 12.sp,
                color = MaterialTheme.colorScheme.primary,
            )
            Text(
                "${offer.name} · ${humanSize(offer.size)}",
                fontSize = 12.sp,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
        TextButton(onClick = onAccept, shape = FieldShape) { Text("接受") }
        TextButton(onClick = onReject, shape = FieldShape) {
            Text("拒绝", color = MaterialTheme.colorScheme.error)
        }
    }
}

@Composable
private fun TransferRow(transfer: Transfer) {
    val percent = if (transfer.total <= 0) 0f else {
        (transfer.done.toFloat() / transfer.total.toFloat()).coerceIn(0f, 1f)
    }
    Column(
        Modifier
            .fillMaxWidth()
            .padding(horizontal = 10.dp, vertical = 3.dp),
    ) {
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
            Text(
                "${if (transfer.outgoing) "↑" else "↓"} ${transfer.name}",
                fontSize = 11.sp,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                modifier = Modifier.weight(1f),
            )
            Text("${(percent * 100).toInt()}%", fontSize = 11.sp)
        }
        LinearProgressIndicator(
            progress = { percent },
            modifier = Modifier
                .fillMaxWidth()
                .height(4.dp),
        )
    }
}

@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun MessageRow(
    message: ChatMessage,
    playingVoice: String?,
    onReply: () -> Unit,
    onPlayVoice: (String) -> Unit,
) {
    if (message.system) {
        Text(
            message.text,
            modifier = Modifier
                .fillMaxWidth()
                .padding(vertical = 2.dp),
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            fontSize = 12.sp,
            textAlign = TextAlign.Center,
        )
        return
    }

    val bubbleColor = if (message.mine) {
        MaterialTheme.colorScheme.primaryContainer
    } else {
        MaterialTheme.colorScheme.surfaceVariant
    }
    val contentColor = if (message.mine) {
        MaterialTheme.colorScheme.onPrimaryContainer
    } else {
        MaterialTheme.colorScheme.onSurface
    }

    Column(
        modifier = Modifier.fillMaxWidth(),
        horizontalAlignment = if (message.mine) Alignment.End else Alignment.Start,
    ) {
        if (!message.mine) {
            Text(
                message.nick,
                modifier = Modifier.padding(start = 6.dp, bottom = 2.dp),
                color = nickColor(message.nick),
                fontSize = 12.sp,
                fontWeight = FontWeight.SemiBold,
            )
        }
        Surface(
            color = bubbleColor,
            shape = BubbleShape,
            modifier = Modifier
                .widthIn(max = 330.dp)
                .then(
                    if (message.mentionsMe) {
                        Modifier.border(1.dp, AsaWarning, BubbleShape)
                    } else {
                        Modifier
                    },
                )
                .pointerInput(message.key) {
                    val threshold = 72.dp.toPx()
                    awaitEachGesture {
                        val down = awaitFirstDown(
                            requireUnconsumed = false,
                            pass = PointerEventPass.Initial,
                        )
                        var total = 0f
                        var dragging = false
                        while (true) {
                            val event = awaitPointerEvent(PointerEventPass.Initial)
                            val change = event.changes.firstOrNull { it.id == down.id } ?: break
                            if (!change.pressed) {
                                if (dragging && total <= -threshold) onReply()
                                break
                            }
                            total += change.position.x - change.previousPosition.x
                            if (!dragging && kotlin.math.abs(total) > viewConfiguration.touchSlop) {
                                dragging = true
                            }
                            if (dragging) {
                                change.consume()
                            }
                        }
                    }
                },
        ) {
            Column(Modifier.padding(horizontal = 10.dp, vertical = 8.dp)) {
                message.reply?.let { reply ->
                    Column(
                        Modifier
                            .fillMaxWidth()
                            .background(
                                MaterialTheme.colorScheme.background.copy(alpha = 0.45f),
                                FieldShape,
                            )
                            .padding(horizontal = 7.dp, vertical = 5.dp),
                    ) {
                        Text(
                            reply.nick,
                            fontSize = 10.sp,
                            color = MaterialTheme.colorScheme.primary,
                        )
                        Text(
                            reply.preview,
                            fontSize = 11.sp,
                            maxLines = 2,
                            overflow = TextOverflow.Ellipsis,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                    Spacer(Modifier.height(5.dp))
                }
                val voicePath = message.voicePath
                if (voicePath != null) {
                    val playing = playingVoice == voicePath
                    Row(
                        modifier = Modifier
                            .fillMaxWidth()
                            .clickable { onPlayVoice(voicePath) }
                            .padding(vertical = 2.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        Icon(
                            if (playing) Icons.Filled.Stop else Icons.Filled.PlayArrow,
                            contentDescription = if (playing) "停止播放" else "播放语音",
                            tint = contentColor,
                            modifier = Modifier.size(22.dp),
                        )
                        Spacer(Modifier.size(6.dp))
                        Text(
                            if (playing) "播放中… 点击停止" else "语音消息 · 点击播放",
                            color = contentColor,
                            fontSize = 15.sp,
                        )
                    }
                } else {
                    SelectionContainer {
                        Text(message.text, color = contentColor, fontSize = 15.sp)
                    }
                }
                Text(
                    timeText(message.ts),
                    modifier = Modifier
                        .align(Alignment.End)
                        .padding(top = 2.dp),
                    fontSize = 10.sp,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
        if (message.mentionsMe) {
            Text(
                "提到了你",
                modifier = Modifier.padding(start = 4.dp, top = 2.dp),
                color = AsaWarning,
                fontSize = 10.sp,
            )
        }
    }
}

private fun timeText(ts: Long): String = runCatching {
    Instant.ofEpochMilli(ts).atZone(ZoneId.systemDefault()).format(TimeFormat)
}.getOrDefault("")

private fun nickColor(nick: String): Color {
    val palette = listOf(
        Color(0xFF7BD88F),
        Color(0xFFFFD166),
        Color(0xFF9B8CFF),
        Color(0xFF6FC3FF),
        Color(0xFFFF9F9F),
        Color(0xFF7FE0D4),
    )
    return palette[(nick.hashCode() and 0x7FFFFFFF) % palette.size]
}

private fun humanSize(bytes: Long): String {
    if (bytes < 1024) return "$bytes B"
    val kb = bytes / 1024.0
    if (kb < 1024) return String.format("%.1f KB", kb)
    val mb = kb / 1024.0
    if (mb < 1024) return String.format("%.1f MB", mb)
    return String.format("%.1f GB", mb / 1024.0)
}

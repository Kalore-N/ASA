package com.asa.chat.ui

import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
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
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.Link
import androidx.compose.material.icons.filled.MoreVert
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
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.asa.chat.AsaViewModel
import com.asa.chat.R
import com.asa.chat.ChatMessage
import com.asa.chat.FileOffer
import com.asa.chat.Member
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
            )
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
                MessageRow(message = message, onReply = { vm.setReply(message) })
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
private fun Composer(
    state: UiState,
    input: TextFieldValue,
    onInputChange: (TextFieldValue) -> Unit,
    onSend: () -> Unit,
    onCancelReply: () -> Unit,
    onAccept: (FileOffer) -> Unit,
    onReject: (FileOffer) -> Unit,
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
            Spacer(Modifier.size(6.dp))
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
private fun MessageRow(message: ChatMessage, onReply: () -> Unit) {
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
                .combinedClickable(onClick = {}, onLongClick = onReply),
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
                Text(message.text, color = contentColor, fontSize = 15.sp)
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

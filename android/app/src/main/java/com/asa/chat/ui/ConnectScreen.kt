package com.asa.chat.ui

import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.systemBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
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
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.foundation.text.KeyboardOptions
import com.asa.chat.LanChannel
import com.asa.chat.R
import com.asa.chat.UiState

private val FieldShape = RoundedCornerShape(6.dp)

@Composable
fun ConnectScreen(
    state: UiState,
    initialPort: Int,
    initialMode: String = "create",
    onStart: (
        mode: String,
        nick: String,
        invite: String?,
        port: String,
        connect: String,
        joinPassword: String,
    ) -> Unit,
    onOpenGroups: () -> Unit,
    onRefreshLan: () -> Unit,
    onJoinLan: (LanChannel, String) -> Unit,
    onOpenSettings: () -> Unit,
) {
    var mode by rememberSaveable { mutableStateOf(if (initialMode == "join") "join" else "create") }
    var nick by rememberSaveable { mutableStateOf(state.nick) }
    var port by rememberSaveable { mutableStateOf(initialPort.toString()) }
    var invite by rememberSaveable { mutableStateOf("") }
    var connect by rememberSaveable { mutableStateOf("") }
    var joinPassword by rememberSaveable { mutableStateOf("") }
    var lanTarget by remember { mutableStateOf<LanChannel?>(null) }
    var lanPassword by remember { mutableStateOf("") }

    LaunchedEffect(mode) {
        if (mode == "join") onRefreshLan()
    }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .background(MaterialTheme.colorScheme.background)
            .systemBarsPadding()
            .navigationBarsPadding()
            .imePadding()
            .verticalScroll(rememberScrollState())
            .padding(horizontal = 22.dp, vertical = 28.dp),
        verticalArrangement = Arrangement.spacedBy(14.dp),
    ) {
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.SpaceBetween,
            verticalAlignment = Alignment.Top,
        ) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Image(
                    painter = painterResource(R.drawable.asa_logo),
                    contentDescription = "ASA logo",
                    modifier = Modifier
                        .size(56.dp)
                        .clip(RoundedCornerShape(8.dp)),
                )
                Spacer(Modifier.width(12.dp))
                Column {
                    Text(
                        text = "ASA",
                        color = MaterialTheme.colorScheme.primary,
                        fontSize = 40.sp,
                        fontWeight = FontWeight.Bold,
                    )
                    Text(
                        text = "去中心化频道 · 没有中心服务器",
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        fontSize = 13.sp,
                    )
                }
            }
            IconButton(onClick = onOpenSettings) {
                Icon(Icons.Filled.Settings, contentDescription = "设置")
            }
        }

        Spacer(Modifier.height(6.dp))

        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            ModeButton("创建群", mode == "create") { mode = "create" }
            ModeButton("加入群", mode == "join") { mode = "join" }
            OutlinedButton(
                onClick = onOpenGroups,
                shape = FieldShape,
                contentPadding = androidx.compose.foundation.layout.PaddingValues(
                    horizontal = 14.dp,
                    vertical = 8.dp,
                ),
            ) {
                Text("频道列表", color = MaterialTheme.colorScheme.onSurfaceVariant, fontSize = 13.sp)
            }
        }

        OutlinedTextField(
                value = nick,
                onValueChange = { nick = it },
                modifier = Modifier.fillMaxWidth(),
                label = { Text("昵称") },
                placeholder = { Text("例如 alice") },
                singleLine = true,
                shape = FieldShape,
            )

        if (mode == "join") {
            OutlinedTextField(
                value = invite,
                onValueChange = { invite = it },
                modifier = Modifier.fillMaxWidth(),
                label = { Text("邀请码") },
                placeholder = { Text("asa://...") },
                minLines = 3,
                maxLines = 5,
                shape = FieldShape,
            )

            Row(
                modifier = Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.SpaceBetween,
            ) {
                Text("局域网频道", fontSize = 13.sp, fontWeight = FontWeight.SemiBold)
                Row(verticalAlignment = Alignment.CenterVertically) {
                    if (state.lanLoading) {
                        Text(
                            "搜索中…",
                            fontSize = 11.sp,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                    TextButton(onClick = onRefreshLan, enabled = !state.lanLoading) {
                        Text("刷新")
                    }
                }
            }
            if (state.lanChannels.isEmpty()) {
                Text(
                    if (state.lanLoading) "正在搜索局域网频道…" else "未发现局域网频道；确认在同一 Wi-Fi 且宿主已开启频道。",
                    fontSize = 12.sp,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            } else {
                state.lanChannels.take(6).forEach { channel ->
                    Surface(
                        modifier = Modifier
                            .fillMaxWidth()
                            .clickable {
                                if (channel.open) {
                                    onJoinLan(channel, "")
                                } else {
                                    lanTarget = channel
                                    lanPassword = ""
                                }
                            },
                        shape = FieldShape,
                        color = MaterialTheme.colorScheme.surfaceVariant,
                    ) {
                        Row(
                            modifier = Modifier.padding(horizontal = 12.dp, vertical = 10.dp),
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            Column(Modifier.weight(1f)) {
                                Text(
                                    channel.name,
                                    fontWeight = FontWeight.SemiBold,
                                    maxLines = 1,
                                    fontSize = 14.sp,
                                )
                                Text(
                                    buildString {
                                        append(channel.host.ifBlank { "未知宿主" })
                                        append(" · ")
                                        append(channel.room.take(8))
                                        if (!channel.open) append(" · 需要密码")
                                    },
                                    fontSize = 11.sp,
                                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                                )
                            }
                            Text(if (channel.open) "加入" else "解锁", fontSize = 12.sp)
                        }
                    }
                }
            }
            HorizontalDivider()
        }

        if (mode == "create") {
            OutlinedTextField(
                value = joinPassword,
                onValueChange = { joinPassword = it },
                modifier = Modifier.fillMaxWidth(),
                label = { Text("频道加入密码（可选）") },
                supportingText = {
                    Text("设置后局域网内需要密码；留空则局域网内可直接加入")
                },
                singleLine = true,
                shape = FieldShape,
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password),
            )
        }

        OutlinedTextField(
            value = port,
            onValueChange = { port = it.filter(Char::isDigit).take(5) },
            modifier = Modifier.fillMaxWidth(),
            label = { Text("监听端口") },
            supportingText = { Text("默认 7777，需要与对方可达") },
            singleLine = true,
            shape = FieldShape,
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number),
        )

        OutlinedTextField(
            value = connect,
            onValueChange = { connect = it },
            modifier = Modifier.fillMaxWidth(),
            label = { Text("手动连接地址（可选）") },
            placeholder = { Text("192.168.1.8:7777") },
            supportingText = { Text("多个地址用逗号分隔") },
            singleLine = true,
            shape = FieldShape,
        )

        if (state.error != null) {
            Text(
                text = state.error,
                color = MaterialTheme.colorScheme.error,
                fontSize = 13.sp,
            )
        }

        Button(
            onClick = {
                onStart(
                    mode,
                    nick,
                    if (mode == "join") invite else null,
                    port,
                    connect,
                    if (mode == "create") joinPassword else "",
                )
            },
            modifier = Modifier
                .fillMaxWidth()
                .height(46.dp),
            shape = FieldShape,
            enabled = !state.connecting,
            colors = ButtonDefaults.buttonColors(
                containerColor = MaterialTheme.colorScheme.primary,
                contentColor = MaterialTheme.colorScheme.onPrimary,
            ),
        ) {
            if (state.connecting) {
                CircularProgressIndicator(
                    modifier = Modifier.height(18.dp),
                    strokeWidth = 2.dp,
                    color = MaterialTheme.colorScheme.onPrimary,
                )
            } else {
                Text(
                    if (mode == "join") "加入频道" else "创建频道",
                    fontWeight = FontWeight.SemiBold,
                )
            }
        }

        Text(
            text = "提示：同一局域网可直接加入；跨网需要可访问的地址。\n" +
                "创建群后，把邀请码发给朋友即可。",
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            fontSize = 12.sp,
            textAlign = TextAlign.Start,
        )
    }

    lanTarget?.let { target ->
        AlertDialog(
            onDismissRequest = { lanTarget = null },
            shape = FieldShape,
            title = { Text("加入「${target.name}」") },
            text = {
                OutlinedTextField(
                    value = lanPassword,
                    onValueChange = { lanPassword = it },
                    singleLine = true,
                    shape = FieldShape,
                    label = { Text("频道加入密码") },
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password),
                )
            },
            confirmButton = {
                TextButton(onClick = {
                    lanTarget = null
                    onJoinLan(target, lanPassword)
                }) { Text("加入") }
            },
            dismissButton = {
                TextButton(onClick = { lanTarget = null }) { Text("取消") }
            },
        )
    }
}

@Composable
private fun ModeButton(text: String, selected: Boolean, onClick: () -> Unit) {
    if (selected) {
        Button(
            onClick = onClick,
            shape = FieldShape,
            contentPadding = androidx.compose.foundation.layout.PaddingValues(horizontal = 14.dp, vertical = 8.dp),
        ) {
            Text(text, fontSize = 13.sp)
        }
    } else {
        OutlinedButton(
            onClick = onClick,
            shape = FieldShape,
            contentPadding = androidx.compose.foundation.layout.PaddingValues(horizontal = 14.dp, vertical = 8.dp),
        ) {
            Text(text, color = MaterialTheme.colorScheme.onSurfaceVariant, fontSize = 13.sp)
        }
    }
}

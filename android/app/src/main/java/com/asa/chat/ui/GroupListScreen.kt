package com.asa.chat.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Edit
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.asa.chat.LanChannel
import com.asa.chat.SavedGroup
import com.asa.chat.UiState
import kotlinx.coroutines.delay
import java.time.Instant
import java.time.ZoneId
import java.time.format.DateTimeFormatter

private val FieldShape = RoundedCornerShape(6.dp)
private val ItemShape = RoundedCornerShape(8.dp)
private val TimeFormat = DateTimeFormatter.ofPattern("MM-dd HH:mm")

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun GroupListScreen(
    state: UiState,
    onEnter: (String) -> Unit,
    onCreate: () -> Unit,
    onJoin: () -> Unit,
    onRename: (String, String) -> Unit,
    onToggleEasytier: (String, Boolean) -> Unit,
    onRefreshLan: () -> Unit,
    onJoinLan: (LanChannel, String) -> Unit,
    onOpenSettings: () -> Unit,
) {
    var renameTarget by remember { mutableStateOf<SavedGroup?>(null) }
    var lanTarget by remember { mutableStateOf<LanChannel?>(null) }
    var lanPassword by remember { mutableStateOf("") }

    LaunchedEffect(Unit) {
        while (true) {
            onRefreshLan()
            delay(4000)
        }
    }

    Scaffold(
        containerColor = MaterialTheme.colorScheme.background,
        topBar = {
            TopAppBar(
                colors = TopAppBarDefaults.topAppBarColors(
                    containerColor = MaterialTheme.colorScheme.surface,
                    titleContentColor = MaterialTheme.colorScheme.onSurface,
                ),
                title = { Text("选择频道", fontWeight = FontWeight.SemiBold) },
                actions = {
                    IconButton(onClick = onOpenSettings) {
                        Icon(Icons.Filled.Settings, contentDescription = "设置")
                    }
                },
            )
        },
    ) { padding ->
        Column(
            modifier = Modifier
                .fillMaxSize()
                .padding(padding)
                .padding(horizontal = 16.dp, vertical = 12.dp),
            verticalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            if (state.lanChannels.isNotEmpty() || state.lanLoading) {
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.SpaceBetween,
                ) {
                    Text("局域网频道", fontSize = 13.sp, fontWeight = FontWeight.SemiBold)
                    if (state.lanLoading) {
                        Text("搜索中…", fontSize = 11.sp, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                }
                state.lanChannels.take(4).forEach { channel ->
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
                        shape = ItemShape,
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
                                    overflow = TextOverflow.Ellipsis,
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
                if (state.lanChannels.isEmpty()) {
                    Text(
                        "正在搜索局域网频道…",
                        fontSize = 12.sp,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                HorizontalDivider()
            }

            if (state.groups.isEmpty()) {
                Text(
                    "还没有保存的频道。",
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    fontSize = 14.sp,
                )
                Spacer(Modifier.height(4.dp))
            } else {
                Text(
                    "点击名称进入；右侧开关控制此频道的 EasyTier 内网穿透。",
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    fontSize = 12.sp,
                )
                LazyColumn(
                    modifier = Modifier.weight(1f),
                    verticalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    items(state.groups, key = { it.id }) { group ->
                        GroupRow(
                            group = group,
                            lanDiscovered = state.lanChannels.any { it.room == group.id },
                            onEnter = { onEnter(group.id) },
                            onRename = { renameTarget = group },
                            onToggleEasytier = { enabled ->
                                onToggleEasytier(group.id, enabled)
                            },
                        )
                    }
                }
            }

            HorizontalDivider()
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Button(
                    onClick = onCreate,
                    modifier = Modifier
                        .weight(1f)
                        .height(44.dp),
                    shape = FieldShape,
                ) { Text("创建频道") }
                OutlinedButton(
                    onClick = onJoin,
                    modifier = Modifier
                        .weight(1f)
                        .height(44.dp),
                    shape = FieldShape,
                ) { Text("加入频道") }
            }
        }
    }

    renameTarget?.let { target ->
        var name by remember(target.id) { mutableStateOf(target.name) }
        AlertDialog(
            onDismissRequest = { renameTarget = null },
            shape = ItemShape,
            title = { Text("频道名称（仅本机）") },
            text = {
                OutlinedTextField(
                    value = name,
                    onValueChange = { name = it },
                    singleLine = true,
                    shape = FieldShape,
                    label = { Text("名称") },
                )
            },
            confirmButton = {
                TextButton(onClick = {
                    onRename(target.id, name)
                    renameTarget = null
                }) { Text("保存") }
            },
            dismissButton = {
                TextButton(onClick = { renameTarget = null }) { Text("取消") }
            },
        )
    }

    lanTarget?.let { target ->
        AlertDialog(
            onDismissRequest = { lanTarget = null },
            shape = ItemShape,
            title = { Text("加入「${target.name}」") },
            text = {
                OutlinedTextField(
                    value = lanPassword,
                    onValueChange = { lanPassword = it },
                    singleLine = true,
                    shape = FieldShape,
                    label = { Text("频道加入密码") },
                    keyboardOptions = androidx.compose.foundation.text.KeyboardOptions(
                        keyboardType = androidx.compose.ui.text.input.KeyboardType.Password,
                    ),
                )
            },
            confirmButton = {
                TextButton(onClick = {
                    val targetChannel = target
                    lanTarget = null
                    onJoinLan(targetChannel, lanPassword)
                }) { Text("加入") }
            },
            dismissButton = {
                TextButton(onClick = { lanTarget = null }) { Text("取消") }
            },
        )
    }
}

@Composable
private fun GroupRow(
    group: SavedGroup,
    lanDiscovered: Boolean,
    onEnter: () -> Unit,
    onRename: () -> Unit,
    onToggleEasytier: (Boolean) -> Unit,
) {
    Surface(
        modifier = Modifier
            .fillMaxWidth()
            .clickable(onClick = onEnter),
        shape = ItemShape,
        color = if (group.active) {
            MaterialTheme.colorScheme.primaryContainer
        } else {
            MaterialTheme.colorScheme.surfaceVariant
        },
    ) {
        Row(
            modifier = Modifier.padding(horizontal = 14.dp, vertical = 12.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Column(Modifier.weight(1f)) {
                Text(
                    group.name,
                    fontWeight = FontWeight.SemiBold,
                    fontSize = 16.sp,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                Text(
                    buildString {
                        append("频道 ")
                        append(group.id.take(8))
                        if (group.easytierEnabled) append(" · EasyTier")
                        if (lanDiscovered) append(" · 局域网已发现")
                        if (group.active) append(" · 当前频道")
                        if (group.lastUsed > 0) {
                            append(" · ")
                            append(
                                runCatching {
                                    Instant.ofEpochMilli(group.lastUsed)
                                        .atZone(ZoneId.systemDefault())
                                        .format(TimeFormat)
                                }.getOrDefault(""),
                            )
                        }
                    },
                    fontSize = 11.sp,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
            Switch(
                checked = group.easytierEnabled,
                onCheckedChange = onToggleEasytier,
                modifier = Modifier.padding(horizontal = 4.dp),
            )
            IconButton(onClick = onRename) {
                Icon(Icons.Filled.Edit, contentDescription = "修改名称")
            }
        }
    }
}

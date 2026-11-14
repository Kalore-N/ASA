package com.asa.chat.ui

import android.content.Context
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.ArrowBack
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Slider
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.asa.chat.AppConfig
import com.asa.chat.ConfigStore

private val FieldShape = RoundedCornerShape(6.dp)

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SettingsScreen(
    context: Context,
    config: AppConfig,
    onSave: (AppConfig) -> Unit,
    onBack: () -> Unit,
) {
    var downloadDir by remember { mutableStateOf(config.downloadDir) }
    var maxFile by remember { mutableStateOf(config.maxFileMb.toString()) }
    var theme by remember { mutableStateOf(config.theme) }
    var fontSize by remember { mutableFloatStateOf(config.fontSize.toFloat()) }
    var compact by remember { mutableStateOf(config.compact) }
    var mdns by remember { mutableStateOf(config.mdns) }
    var autoAccept by remember { mutableStateOf(config.autoAccept) }
    var relays by remember { mutableStateOf(config.relays.joinToString("\n")) }
    val clipboard = LocalClipboardManager.current
    val effectivePath = downloadDir.ifBlank { ConfigStore.defaultDownloadDir(context) }

    Scaffold(
        containerColor = MaterialTheme.colorScheme.background,
        topBar = {
            TopAppBar(
                colors = TopAppBarDefaults.topAppBarColors(
                    containerColor = MaterialTheme.colorScheme.surface,
                    titleContentColor = MaterialTheme.colorScheme.onSurface,
                ),
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(Icons.Filled.ArrowBack, contentDescription = "返回")
                    }
                },
                title = { Text("设置", fontWeight = FontWeight.SemiBold) },
                actions = {
                    TextButton(onClick = {
                        val mb = maxFile.toLongOrNull()?.coerceAtLeast(0) ?: 100
                        onSave(
                            AppConfig(
                                downloadDir = downloadDir.trim(),
                                maxFileMb = mb,
                                autoAccept = autoAccept,
                                theme = theme,
                                fontSize = fontSize.toInt().coerceIn(10, 30),
                                compact = compact,
                                mdns = mdns,
                                relays = relays.split('\n', ',')
                                    .map { it.trim() }
                                    .filter { it.isNotEmpty() },
                            ),
                        )
                    }) { Text("保存") }
                },
            )
        },
    ) { padding ->
        Column(
            modifier = Modifier
                .fillMaxSize()
                .padding(padding)
                .verticalScroll(rememberScrollState())
                .padding(horizontal = 18.dp, vertical = 12.dp),
            verticalArrangement = Arrangement.spacedBy(14.dp),
        ) {
            Text("文件", color = MaterialTheme.colorScheme.primary, fontSize = 13.sp)
            OutlinedTextField(
                value = downloadDir,
                onValueChange = { downloadDir = it },
                modifier = Modifier.fillMaxWidth(),
                label = { Text("下载保存位置") },
                placeholder = { Text(ConfigStore.defaultDownloadDir(context)) },
                supportingText = {
                    Text(
                        "留空使用应用默认目录；当前：$effectivePath",
                        maxLines = 2,
                        overflow = TextOverflow.Ellipsis,
                    )
                },
                shape = FieldShape,
            )
            TextButton(
                onClick = { clipboard.setText(AnnotatedString(effectivePath)) },
                shape = FieldShape,
            ) { Text("复制保存路径") }

            OutlinedTextField(
                value = maxFile,
                onValueChange = { maxFile = it.filter(Char::isDigit).take(7) },
                modifier = Modifier.fillMaxWidth(),
                label = { Text("单文件上限（MB）") },
                supportingText = { Text("0 表示不限制，默认 100 MB") },
                singleLine = true,
                shape = FieldShape,
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number),
            )

            HorizontalDivider()
            Text("界面", color = MaterialTheme.colorScheme.primary, fontSize = 13.sp)
            Text("主题", fontSize = 13.sp)
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                ThemeButton("深色", theme == "dark") { theme = "dark" }
                ThemeButton("浅色", theme == "light") { theme = "light" }
                ThemeButton("高对比", theme == "high-contrast") { theme = "high-contrast" }
            }

            Text("字体大小：${fontSize.toInt()}", fontSize = 13.sp)
            Slider(
                value = fontSize,
                onValueChange = { fontSize = it },
                valueRange = 10f..24f,
                steps = 13,
            )

            SettingSwitch(
                title = "紧凑布局",
                checked = compact,
                onCheckedChange = { compact = it },
            )

            HorizontalDivider()
            Text("网络", color = MaterialTheme.colorScheme.primary, fontSize = 13.sp)
            SettingSwitch(
                title = "局域网自动发现（mDNS）",
                checked = mdns,
                onCheckedChange = { mdns = it },
            )
            OutlinedTextField(
                value = relays,
                onValueChange = { relays = it },
                modifier = Modifier.fillMaxWidth(),
                label = { Text("中继地址（可选）") },
                placeholder = { Text("relay.example.com:9000") },
                supportingText = { Text("每行一个；跨网时使用，局域网可不填") },
                minLines = 2,
                maxLines = 6,
                shape = FieldShape,
            )
            SettingSwitch(
                title = "自动接受文件",
                checked = autoAccept,
                onCheckedChange = { autoAccept = it },
            )

            Text(
                "提示：下载目录、大小上限、mDNS、中继在下次进入频道时生效；主题和字号立即生效。",
                fontSize = 12.sp,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            Text(
                "频道内昵称可在聊天页菜单的「修改昵称」中设置，会单独保存在当前频道。",
                fontSize = 12.sp,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}

@Composable
private fun ThemeButton(text: String, selected: Boolean, onClick: () -> Unit) {
    if (selected) {
        androidx.compose.material3.Button(
            onClick = onClick,
            shape = FieldShape,
            contentPadding = androidx.compose.foundation.layout.PaddingValues(
                horizontal = 12.dp,
                vertical = 6.dp,
            ),
        ) { Text(text, fontSize = 12.sp) }
    } else {
        OutlinedButton(
            onClick = onClick,
            shape = FieldShape,
            contentPadding = androidx.compose.foundation.layout.PaddingValues(
                horizontal = 12.dp,
                vertical = 6.dp,
            ),
        ) { Text(text, fontSize = 12.sp) }
    }
}

@Composable
private fun SettingSwitch(title: String, checked: Boolean, onCheckedChange: (Boolean) -> Unit) {
    Row(
        modifier = Modifier.fillMaxWidth(),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.SpaceBetween,
    ) {
        Text(title, fontSize = 13.sp)
        Switch(checked = checked, onCheckedChange = onCheckedChange)
    }
}

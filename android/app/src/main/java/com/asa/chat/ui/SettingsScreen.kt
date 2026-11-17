package com.asa.chat.ui

import android.content.Context
import android.widget.Toast
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.ArrowBack
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.Surface
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.RadioButton
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
import androidx.compose.runtime.rememberCoroutineScope
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
import androidx.core.content.pm.PackageInfoCompat
import com.asa.chat.AppConfig
import com.asa.chat.AsaVpnService
import com.asa.chat.ConfigStore
import com.asa.chat.EasytierStore
import com.asa.chat.NativeBridge
import com.asa.chat.UpdateChecker
import com.asa.chat.UpdateInfo
import org.json.JSONObject
import java.io.File
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

private val FieldShape = RoundedCornerShape(6.dp)

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SettingsScreen(
    context: Context,
    config: AppConfig,
    roomId: String,
    etSelected: String,
    etEnabled: Boolean,
    etIp: String?,
    etError: String?,
    onSelectEtNode: (String) -> Unit,
    onRemoveEtNode: (String) -> Unit,
    onRestartChannel: () -> Unit,
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
    var etNodes by remember { mutableStateOf(EasytierStore.load(context)) }
    var etInput by remember { mutableStateOf("") }
    var etProbing by remember { mutableStateOf<Set<String>>(emptySet()) }
    var etProbeResults by remember { mutableStateOf<Map<String, String>>(emptyMap()) }
    val etEffectiveSelected = etSelected.ifBlank { etNodes.firstOrNull().orEmpty() }
    val scope = rememberCoroutineScope()

    fun probeNode(node: String) {
        if (node in etProbing) return
        etProbing = etProbing + node
        scope.launch {
            val result = withContext(Dispatchers.IO) { EasytierStore.probe(node) }
            etProbeResults = etProbeResults + (
                node to result.fold(
                    onSuccess = { "通 · $it ms" },
                    onFailure = { "不通 · ${it.message ?: "未知错误"}" },
                )
                )
            etProbing = etProbing - node
        }
    }
    var voiceEnabled by remember { mutableStateOf(config.voiceEnabled) }
    var voicePushToTalk by remember { mutableStateOf(config.voicePushToTalk) }
    var voiceRecordDir by remember { mutableStateOf(config.voiceRecordDir) }
    val clipboard = LocalClipboardManager.current
    val effectivePath = downloadDir.ifBlank { ConfigStore.defaultDownloadDir(context) }
    val packageInfo = remember {
        runCatching { context.packageManager.getPackageInfo(context.packageName, 0) }.getOrNull()
    }
    val appVersion = packageInfo?.versionName ?: "unknown"
    val appCode = packageInfo?.let { PackageInfoCompat.getLongVersionCode(it) } ?: 0L
    val coreVersion = remember {
        runCatching { JSONObject(NativeBridge.version()).optString("version") }.getOrDefault("")
    }
    var checkingUpdate by remember { mutableStateOf(false) }
    var updateInfo by remember { mutableStateOf<UpdateInfo?>(null) }
    var updateNotice by remember { mutableStateOf<String?>(null) }
    var updateError by remember { mutableStateOf<String?>(null) }
    var downloading by remember { mutableStateOf(false) }
    var downloadDone by remember { mutableStateOf(0L) }
    var downloadTotal by remember { mutableStateOf(0L) }
    var downloadedFile by remember { mutableStateOf<File?>(null) }
    var installNotice by remember { mutableStateOf<String?>(null) }

    val checkUpdate: () -> Unit = {
        if (!checkingUpdate && !downloading) {
            checkingUpdate = true
            updateInfo = null
            updateNotice = null
            updateError = null
            installNotice = null
            scope.launch {
                val result = UpdateChecker.check()
                checkingUpdate = false
                result.onSuccess { info ->
                    if (UpdateChecker.isNewer(info.version, appVersion)) {
                        updateInfo = info
                    } else {
                        updateNotice = "已是最新版本（$appVersion）"
                    }
                }.onFailure { error ->
                    updateError = "检查更新失败：${updateErrorText(error)}"
                }
            }
        }
    }

    val downloadUpdate: () -> Unit = {
        val info = updateInfo
        if (info != null && !downloading) {
            if (info.apkUrl == null) {
                UpdateChecker.openReleasePage(context, info.pageUrl)
            } else {
                downloading = true
                downloadDone = 0L
                downloadTotal = 0L
                downloadedFile = null
                installNotice = null
                updateError = null
                scope.launch {
                    val result = runCatching {
                        UpdateChecker.download(context, info) { _, done, total ->
                            downloadDone = done
                            downloadTotal = total
                        }
                    }
                    downloading = false
                    result.onSuccess { file ->
                        downloadedFile = file
                        if (!UpdateChecker.canInstall(context)) {
                            installNotice = "下载完成。请允许「安装未知应用」，然后点击「安装」。"
                            UpdateChecker.requestInstallPermission(context)
                        } else {
                            runCatching { UpdateChecker.install(context, file) }
                                .onFailure {
                                    installNotice = "下载完成，但无法启动安装器：${it.message}"
                                }
                        }
                    }.onFailure { error ->
                        updateError = "下载失败：${updateErrorText(error)}"
                    }
                }
            }
        }
    }

    val installDownloaded: () -> Unit = {
        val file = downloadedFile
        if (file != null) {
            if (!UpdateChecker.canInstall(context)) {
                installNotice = "请先允许「安装未知应用」，然后点击「安装」。"
                UpdateChecker.requestInstallPermission(context)
            } else {
                runCatching { UpdateChecker.install(context, file) }
                    .onFailure { installNotice = "无法启动安装器：${it.message}" }
            }
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
                                voiceEnabled = voiceEnabled,
                                voicePushToTalk = voicePushToTalk,
                                voiceRecordDir = voiceRecordDir.trim(),
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
            Text("语音", color = MaterialTheme.colorScheme.primary, fontSize = 13.sp)
            SettingSwitch(
                title = "启用语音通话和语音消息",
                checked = voiceEnabled,
                onCheckedChange = { voiceEnabled = it },
            )
            SettingSwitch(
                title = "TUI 使用按住说话",
                checked = voicePushToTalk,
                onCheckedChange = { voicePushToTalk = it },
            )
            OutlinedTextField(
                value = voiceRecordDir,
                onValueChange = { voiceRecordDir = it },
                modifier = Modifier.fillMaxWidth(),
                label = { Text("语音保存目录（可选）") },
                placeholder = { Text("留空使用下载目录/voice") },
                singleLine = true,
                shape = FieldShape,
            )
            Text(
                "通话使用 PCM 16kHz 单声道；UDP 不通时自动回退 TCP。语音消息保存为 WAV。",
                fontSize = 12.sp,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )

            HorizontalDivider()
            Text("EasyTier 内网穿透", color = MaterialTheme.colorScheme.primary, fontSize = 13.sp)
            if (roomId.isNotBlank()) {
                val etStatus = when {
                    !etEnabled -> "未开启"
                    etIp != null -> "运行中 · 虚拟 IP $etIp"
                    etError != null -> "异常：$etError"
                    else -> "已开启，等待启动"
                }
                Text("EasyTier 状态：$etStatus", fontSize = 12.sp)
                Text(
                    "VPN：${AsaVpnService.status}",
                    fontSize = 11.sp,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                OutlinedButton(
                    onClick = onRestartChannel,
                    shape = FieldShape,
                ) { Text("重新连接频道", fontSize = 12.sp) }
            }
            Text(
                "节点地址只用于发现同一个 EasyTier 网络；网络名和密钥由频道密钥派生。",
                fontSize = 12.sp,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            if (etNodes.isEmpty()) {
                Text(
                    "尚未添加节点。没有节点时频道仍可使用局域网和 ASA 中继。",
                    fontSize = 12.sp,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            } else {
                if (roomId.isBlank()) {
                    Text(
                        "进入频道后，可在这里选择本频道使用的节点。",
                        fontSize = 11.sp,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                etNodes.forEach { node ->
                    Row(
                        modifier = Modifier.fillMaxWidth(),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        RadioButton(
                            selected = etEffectiveSelected == node,
                            onClick = {
                                if (roomId.isNotBlank()) onSelectEtNode(node)
                            },
                            enabled = roomId.isNotBlank(),
                        )
                        Column(Modifier.weight(1f)) {
                            Text(
                                node,
                                fontSize = 12.sp,
                                maxLines = 1,
                                overflow = TextOverflow.Ellipsis,
                            )
                            val result = etProbeResults[node]
                            if (result != null) {
                                Text(
                                    result,
                                    fontSize = 11.sp,
                                    color = if (result.startsWith("通")) {
                                        MaterialTheme.colorScheme.primary
                                    } else {
                                        MaterialTheme.colorScheme.error
                                    },
                                )
                            }
                        }
                        TextButton(
                            onClick = { probeNode(node) },
                            enabled = node !in etProbing,
                        ) {
                            Text(if (node in etProbing) "测试中…" else "测试", fontSize = 12.sp)
                        }
                        IconButton(onClick = {
                            onRemoveEtNode(node)
                            etNodes = EasytierStore.load(context)
                            etProbeResults = etProbeResults - node
                        }) {
                            Icon(Icons.Filled.Delete, contentDescription = "删除节点")
                        }
                    }
                }
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    OutlinedButton(
                        onClick = { etNodes.forEach { probeNode(it) } },
                        shape = FieldShape,
                        enabled = etNodes.any { it !in etProbing },
                    ) { Text("测试全部", fontSize = 12.sp) }
                    if (etProbing.isNotEmpty()) {
                        CircularProgressIndicator(
                            modifier = Modifier.size(16.dp),
                            strokeWidth = 2.dp,
                        )
                    }
                }
                Text(
                    "节点按频道保存选择，改动在重新进入频道后生效；删除会同时清理旧频道里的残留配置。",
                    fontSize = 11.sp,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                OutlinedTextField(
                    value = etInput,
                    onValueChange = { etInput = it },
                    modifier = Modifier.weight(1f),
                    label = { Text("EasyTier 节点") },
                    placeholder = { Text("tcp://host:11010") },
                    singleLine = true,
                    shape = FieldShape,
                )
                Button(
                    onClick = {
                        if (EasytierStore.add(context, etInput)) {
                            etNodes = EasytierStore.load(context)
                            etInput = ""
                        } else {
                            Toast.makeText(
                                context,
                                "节点地址无效（打码地址请从 EasyTier QQ 群获取）",
                                Toast.LENGTH_SHORT,
                            ).show()
                        }
                    },
                    shape = FieldShape,
                ) { Text("添加") }
            }
            OutlinedButton(
                onClick = {
                    scope.launch {
                        val result = withContext(Dispatchers.IO) {
                            EasytierStore.importQtet(context)
                        }
                        result.onSuccess { added ->
                            etNodes = EasytierStore.load(context)
                            Toast.makeText(
                                context,
                                if (added > 0) "已导入 $added 个节点" else "没有新的可用节点",
                                Toast.LENGTH_SHORT,
                            ).show()
                        }.onFailure { error ->
                            Toast.makeText(
                                context,
                                "导入失败: ${error.message}",
                                Toast.LENGTH_LONG,
                            ).show()
                        }
                    }
                },
                shape = FieldShape,
            ) { Text("从 qtet 监控页一键导入") }
            Text(
                "提示：EasyTier 只路由 ASA 自身流量；Android 同一用户同时只能有一个 VpnService。若已有其他 VPN，开启时会提示并可能断开它。工作资料双 VPN 方案见 README。",
                fontSize = 12.sp,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
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

            HorizontalDivider()
            Text("关于", color = MaterialTheme.colorScheme.primary, fontSize = 13.sp)
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.SpaceBetween,
            ) {
                Text("应用版本", fontSize = 13.sp)
                Text(
                    "$appVersion ($appCode)",
                    fontSize = 13.sp,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            if (coreVersion.isNotBlank()) {
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.SpaceBetween,
                ) {
                    Text("核心版本", fontSize = 13.sp)
                    Text(
                        coreVersion,
                        fontSize = 13.sp,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
            Row(
                horizontalArrangement = Arrangement.spacedBy(10.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                OutlinedButton(
                    onClick = checkUpdate,
                    enabled = !checkingUpdate && !downloading,
                    shape = FieldShape,
                ) { Text(if (checkingUpdate) "检查中…" else "检查更新") }
                if (checkingUpdate) {
                    CircularProgressIndicator(
                        modifier = Modifier.size(18.dp),
                        strokeWidth = 2.dp,
                    )
                }
            }
            updateNotice?.let {
                Text(it, fontSize = 12.sp, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
            updateError?.let {
                Text(it, fontSize = 12.sp, color = MaterialTheme.colorScheme.error)
            }
            updateInfo?.let { info ->
                Surface(
                    color = MaterialTheme.colorScheme.surfaceVariant,
                    shape = FieldShape,
                    modifier = Modifier.fillMaxWidth(),
                ) {
                    Column(
                        modifier = Modifier.padding(12.dp),
                        verticalArrangement = Arrangement.spacedBy(6.dp),
                    ) {
                        Text(
                            "发现新版本 ${info.tag}",
                            fontSize = 14.sp,
                            fontWeight = FontWeight.SemiBold,
                        )
                        if (info.publishedAt.isNotBlank()) {
                            Text(
                                "当前 $appVersion · 发布 ${info.publishedAt.take(10)}",
                                fontSize = 11.sp,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                        if (info.notes.isNotBlank()) {
                            Text(
                                info.notes,
                                fontSize = 12.sp,
                                maxLines = 10,
                                overflow = TextOverflow.Ellipsis,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                        if (info.apkUrl == null) {
                            Text(
                                "该版本没有 APK 资产，请到发布页下载。",
                                fontSize = 11.sp,
                                color = MaterialTheme.colorScheme.error,
                            )
                        }
                        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                            Button(
                                onClick = downloadUpdate,
                                enabled = !downloading,
                                shape = FieldShape,
                            ) {
                                Text(if (downloadedFile != null) "重新下载" else "下载并安装")
                            }
                            TextButton(onClick = {
                                UpdateChecker.openReleasePage(context, info.pageUrl)
                            }) { Text("打开发布页") }
                        }
                    }
                }
            }
            if (downloading) {
                val progress = if (downloadTotal > 0) {
                    (downloadDone.toFloat() / downloadTotal.toFloat()).coerceIn(0f, 1f)
                } else {
                    0f
                }
                if (downloadTotal > 0) {
                    LinearProgressIndicator(
                        progress = { progress },
                        modifier = Modifier.fillMaxWidth(),
                    )
                    Text(
                        "正在下载 ${formatBytes(downloadDone)} / ${formatBytes(downloadTotal)}" +
                            "（${(progress * 100).toInt()}%）",
                        fontSize = 11.sp,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                } else {
                    LinearProgressIndicator(modifier = Modifier.fillMaxWidth())
                    Text(
                        "正在下载 ${formatBytes(downloadDone)}",
                        fontSize = 11.sp,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
            downloadedFile?.let { file ->
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.SpaceBetween,
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Text(
                        "已下载 ${file.name}",
                        fontSize = 11.sp,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.weight(1f),
                    )
                    TextButton(onClick = installDownloaded) { Text("安装") }
                }
            }
            installNotice?.let {
                Text(it, fontSize = 12.sp, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
            Text(
                "更新检查只在点击时连接 GitHub API；下载的 APK 保存在应用私有目录，安装时会调用系统安装器。",
                fontSize = 11.sp,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}

private fun updateErrorText(error: Throwable): String = when (error) {
    is java.net.UnknownHostException -> "无法连接 GitHub，请检查网络或代理"
    is java.net.SocketTimeoutException -> "连接 GitHub 超时，请稍后再试"
    else -> error.message ?: error::class.simpleName ?: "未知错误"
}

private fun formatBytes(bytes: Long): String {
    if (bytes < 1024) return "$bytes B"
    val units = listOf("KB", "MB", "GB")
    var value = bytes.toDouble()
    var index = -1
    while (value >= 1024 && index < units.lastIndex) {
        value /= 1024
        index += 1
    }
    return String.format(java.util.Locale.US, "%.1f %s", value, units[index.coerceAtLeast(0)])
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

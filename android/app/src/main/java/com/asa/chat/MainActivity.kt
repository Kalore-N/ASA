package com.asa.chat

import android.Manifest
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.BackHandler
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import com.asa.chat.ui.AsaTheme
import com.asa.chat.ui.ChatScreen
import com.asa.chat.ui.ConnectScreen
import com.asa.chat.ui.GroupListScreen
import com.asa.chat.ui.SettingsScreen

class MainActivity : ComponentActivity() {
    private val notificationPermission = registerForActivityResult(
        ActivityResultContracts.RequestPermission(),
    ) {}

    private val vpnPermission = registerForActivityResult(
        ActivityResultContracts.StartActivityForResult(),
    ) { result ->
        VpnPermission.result(result.resultCode == RESULT_OK)
    }

    override fun onResume() {
        super.onResume()
        AppVisibility.foreground = true
    }

    override fun onPause() {
        AppVisibility.foreground = false
        super.onPause()
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU &&
            checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED
        ) {
            notificationPermission.launch(Manifest.permission.POST_NOTIFICATIONS)
        }
        val activity = this
        VpnPermission.attach { intent -> vpnPermission.launch(intent) }
        setContent {
            var config by remember { mutableStateOf(ConfigStore.load(activity)) }
            var showSettings by remember { mutableStateOf(false) }
            var connectMode by remember { mutableStateOf("create") }

            AsaTheme(theme = config.theme, fontSize = config.fontSize) {
                val vm: AsaViewModel = viewModel()
                val state by vm.state.collectAsStateWithLifecycle()

                BackHandler(enabled = showSettings) { showSettings = false }
                BackHandler(enabled = !showSettings && state.screen == Screen.Chat) {
                    // 返回键回到频道列表（主页），连接保持；只有确认「退出频道」才停止。
                    vm.backToGroups()
                }
                BackHandler(enabled = !showSettings && state.screen == Screen.Groups) {
                    // 频道列表就是主页，再按返回才回到桌面。
                    moveTaskToBack(true)
                }

                if (showSettings) {
                    SettingsScreen(
                        context = activity,
                        config = config,
                        onSave = { updated ->
                            ConfigStore.save(activity, updated)
                            config = updated
                            showSettings = false
                        },
                        onBack = { showSettings = false },
                    )
                } else {
                    when (state.screen) {
                        Screen.Connect -> ConnectScreen(
                            state = state,
                            initialPort = config.port,
                            initialMode = connectMode,
                            onStart = { mode, nick, invite, port, connect, joinPassword ->
                                vm.start(
                                    mode,
                                    nick,
                                    invite,
                                    port,
                                    connect,
                                    config,
                                    joinPassword = joinPassword,
                                )
                            },
                            onOpenGroups = { vm.showGroups() },
                            onOpenSettings = { showSettings = true },
                        )
                        Screen.Groups -> GroupListScreen(
                            state = state,
                            onEnter = { roomId -> vm.enterGroup(roomId) },
                            onCreate = {
                                connectMode = "create"
                                vm.showConnect()
                            },
                            onJoin = {
                                connectMode = "join"
                                vm.showConnect()
                            },
                            onRename = { roomId, name -> vm.renameGroup(roomId, name) },
                            onToggleEasytier = { roomId, enabled ->
                                vm.toggleEasytier(roomId, enabled)
                            },
                            onRefreshLan = { vm.discoverLan() },
                            onJoinLan = { channel, password ->
                                vm.joinLan(channel, password)
                            },
                            onOpenSettings = { showSettings = true },
                        )
                        Screen.Chat -> ChatScreen(
                            state = state,
                            vm = vm,
                            onOpenSettings = { showSettings = true },
                        )
                    }
                }
            }
        }
    }
}

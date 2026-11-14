package com.asa.chat

import android.content.Context
import android.content.Intent
import android.net.VpnService

/**
 * 用 ActivityResult 方式申请系统 VPN 权限。
 *
 * 不能从 Application context 直接 startActivity 了事：系统确认页需要
 * 把结果回传给 Activity，否则确认后 prepare() 仍会返回 Intent。
 */
object VpnPermission {
    private var launcher: ((Intent) -> Unit)? = null
    private var pending: ((Boolean) -> Unit)? = null

    fun attach(launch: (Intent) -> Unit) {
        launcher = launch
    }

    fun request(context: Context, callback: (Boolean) -> Unit) {
        val intent = VpnService.prepare(context)
        if (intent == null) {
            callback(true)
            return
        }
        val launch = launcher
        if (launch == null) {
            callback(false)
            return
        }
        pending = callback
        launch(intent)
    }

    fun result(granted: Boolean) {
        val callback = pending
        pending = null
        callback?.invoke(granted)
    }
}

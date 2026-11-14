package com.asa.chat

import android.Manifest
import android.content.Context
import android.content.pm.PackageManager
import androidx.core.content.ContextCompat

/** 麦克风权限的 ActivityResult 协调器。 */
object MicPermission {
    private var launcher: ((String) -> Unit)? = null
    private var pending: ((Boolean) -> Unit)? = null

    fun attach(launch: (String) -> Unit) {
        launcher = launch
    }

    fun request(context: Context, callback: (Boolean) -> Unit) {
        if (ContextCompat.checkSelfPermission(context, Manifest.permission.RECORD_AUDIO) ==
            PackageManager.PERMISSION_GRANTED
        ) {
            callback(true)
            return
        }
        val launch = launcher
        if (launch == null) {
            callback(false)
            return
        }
        pending = callback
        launch(Manifest.permission.RECORD_AUDIO)
    }

    fun result(granted: Boolean) {
        val callback = pending
        pending = null
        callback?.invoke(granted)
    }
}

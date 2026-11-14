package com.asa.chat

import android.app.Application

class AsaApplication : Application() {
    override fun onCreate() {
        super.onCreate()
        NotificationHelper.createChannels(this)
    }
}

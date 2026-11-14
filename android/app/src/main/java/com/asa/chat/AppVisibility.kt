package com.asa.chat

/** 应用是否在前台；后台时新消息才发系统通知。 */
object AppVisibility {
    @Volatile
    var foreground: Boolean = false
}

package com.easytier.jni

/** EasyTier Android JNI 的最小封装。 */
object EasyTierJNI {
    init {
        // JNI 库通过 extern "C" 调用 easytier-ffi，先加载提供符号的库。
        System.loadLibrary("easytier_ffi")
        System.loadLibrary("easytier_android_jni")
    }

    @JvmStatic external fun setTunFd(instanceName: String, fd: Int): Int

    @JvmStatic external fun parseConfig(config: String): Int

    @JvmStatic external fun runNetworkInstance(config: String): Int

    @JvmStatic external fun retainNetworkInstance(instanceNames: Array<String>?): Int

    @JvmStatic external fun deleteNetworkInstance(instanceName: String): Int

    @JvmStatic external fun collectNetworkInfos(maxLength: Int): String?

    @JvmStatic external fun listInstances(maxLength: Int): String?

    @JvmStatic external fun getLastError(): String?

    @JvmStatic
    fun stopAllInstances(): Int {
        return retainNetworkInstance(null)
    }
}

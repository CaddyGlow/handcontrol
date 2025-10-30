package com.handcontrol

import android.app.Application
import com.handcontrol.core.logging.LoggerInitializer
import dagger.hilt.android.HiltAndroidApp

@HiltAndroidApp
class HandControlApplication : Application() {
    override fun onCreate() {
        super.onCreate()
        LoggerInitializer.init(this)

        // Enable IPv6 support - prefer IPv6 addresses when available
        // This is critical for connecting to servers with IPv6-only addresses
        System.setProperty("java.net.preferIPv6Addresses", "true")
    }
}

package com.handcontrol

import android.app.Application
import com.handcontrol.core.logging.LoggerInitializer
import dagger.hilt.android.HiltAndroidApp

@HiltAndroidApp
class HandControlApplication : Application() {
    override fun onCreate() {
        super.onCreate()
        LoggerInitializer.init(this)
    }
}

package com.handcontrol.core.logging

import android.app.Application
import com.handcontrol.BuildConfig
import timber.log.Timber

object LoggerInitializer {
    @Suppress("UNUSED_PARAMETER")
    fun init(application: Application) {
        if (Timber.forest().isEmpty() && BuildConfig.DEBUG) {
            Timber.plant(Timber.DebugTree())
        }
    }
}

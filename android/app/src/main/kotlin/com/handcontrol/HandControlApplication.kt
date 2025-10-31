package com.handcontrol

import android.app.Application
import androidx.lifecycle.ProcessLifecycleOwner
import androidx.lifecycle.lifecycleScope
import com.handcontrol.core.logging.LogCollectorTree
import com.handcontrol.core.logging.LoggerInitializer
import com.handcontrol.data.settings.LogLevel
import com.handcontrol.data.settings.SettingsRepository
import dagger.hilt.android.HiltAndroidApp
import kotlinx.coroutines.flow.collect
import kotlinx.coroutines.launch
import timber.log.Timber
import javax.inject.Inject

@HiltAndroidApp
class HandControlApplication : Application() {
    @Inject
    lateinit var settingsRepository: SettingsRepository

    private var currentLogTree: Timber.Tree? = null
    private val logCollectorTree = LogCollectorTree()

    override fun onCreate() {
        super.onCreate()

        // Initialize existing logging system (MUST keep this)
        LoggerInitializer.init(this)

        // Enable IPv6 support - prefer IPv6 addresses when available (MUST keep this)
        // This is critical for connecting to servers with IPv6-only addresses
        System.setProperty("java.net.preferIPv6Addresses", "true")

        // Plant log collector for export functionality
        Timber.plant(logCollectorTree)

        // Observe settings and reconfigure logging
        // Use ProcessLifecycleOwner since Application is not a LifecycleOwner
        ProcessLifecycleOwner.get().lifecycleScope.launch {
            settingsRepository.settings.collect { settings ->
                configureLogging(settings.logLevel, settings.enableNetworkLogging)
            }
        }
    }

    private fun configureLogging(level: LogLevel, networkLogging: Boolean) {
        // Remove old tree if exists
        currentLogTree?.let { Timber.uproot(it) }

        // Plant new tree with configured level
        currentLogTree = object : Timber.DebugTree() {
            override fun isLoggable(tag: String?, priority: Int): Boolean {
                return priority >= level.toPriority()
            }

            override fun log(priority: Int, tag: String?, message: String, t: Throwable?) {
                // Optionally suppress network logs based on setting
                if (!networkLogging && tag?.contains("gRPC") == true) {
                    return
                }
                super.log(priority, tag, message, t)
            }
        }.also { Timber.plant(it) }

        if (networkLogging) {
            Timber.w("Network logging enabled - may log sensitive data")
        }
    }

    private fun LogLevel.toPriority(): Int = when (this) {
        LogLevel.VERBOSE -> android.util.Log.VERBOSE
        LogLevel.DEBUG -> android.util.Log.DEBUG
        LogLevel.INFO -> android.util.Log.INFO
        LogLevel.WARN -> android.util.Log.WARN
        LogLevel.ERROR -> android.util.Log.ERROR
    }
}

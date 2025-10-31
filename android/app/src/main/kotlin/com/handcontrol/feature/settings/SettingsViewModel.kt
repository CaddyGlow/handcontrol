package com.handcontrol.feature.settings

import android.content.Context
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.handcontrol.core.logging.LogCollectorTree
import com.handcontrol.data.database.EnrolledServerRepository
import com.handcontrol.data.settings.*
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.launch
import timber.log.Timber
import java.io.File
import javax.inject.Inject

@HiltViewModel
class SettingsViewModel @Inject constructor(
    private val settingsRepository: SettingsRepository,
    private val serverRepository: EnrolledServerRepository
) : ViewModel() {

    val settings: StateFlow<AppSettings> = settingsRepository.settings
        .stateIn(
            scope = viewModelScope,
            started = SharingStarted.WhileSubscribed(5000),
            initialValue = AppSettings()
        )

    fun updateTheme(theme: Theme) {
        viewModelScope.launch {
            settingsRepository.updateTheme(theme)
        }
    }

    fun updateRelayFallback(enabled: Boolean) {
        viewModelScope.launch {
            settingsRepository.updateRelayFallback(enabled)
        }
    }

    fun updateIpv6Preference(preference: IpPreference) {
        viewModelScope.launch {
            settingsRepository.updateIpv6Preference(preference)
        }
    }

    fun updateDirectConnectionTimeout(seconds: Int) {
        viewModelScope.launch {
            settingsRepository.updateDirectConnectionTimeout(seconds)
        }
    }

    fun updateAutoDiscovery(enabled: Boolean) {
        viewModelScope.launch {
            settingsRepository.updateAutoDiscovery(enabled)
        }
    }

    fun updateDiscoveryTimeout(seconds: Int) {
        viewModelScope.launch {
            settingsRepository.updateDiscoveryTimeout(seconds)
        }
    }

    fun updateLogLevel(level: LogLevel) {
        viewModelScope.launch {
            settingsRepository.updateLogLevel(level)
            // Logging configuration is handled by HandControlApplication
        }
    }

    fun updateNetworkLogging(enabled: Boolean) {
        viewModelScope.launch {
            settingsRepository.updateNetworkLogging(enabled)
            // Logging configuration is handled by HandControlApplication
        }
    }

    fun updateNetworkDiagnostics(enabled: Boolean) {
        viewModelScope.launch {
            settingsRepository.updateNetworkDiagnostics(enabled)
        }
    }

    fun clearAllData(onComplete: (Result<Unit>) -> Unit) {
        viewModelScope.launch {
            val result = runCatching {
                // 1. Capture existing servers for logging
                val servers = serverRepository.allServers.first()

                // 2. Delete all enrolled servers (certificates are managed by Android Keystore)
                Timber.d("Deleting %d enrolled servers", servers.size)
                serverRepository.deleteAllServers()

                // 3. Reset settings to defaults
                settingsRepository.resetToDefaults()
                    .getOrThrow() // Propagate error if settings reset fails

                Timber.i("All data cleared successfully")
            }.onFailure { e ->
                Timber.e(e, "Failed to clear all data")
            }

            onComplete(result)
        }
    }

    fun exportLogs(context: Context, onComplete: (Result<File?>) -> Unit) {
        viewModelScope.launch {
            val result = runCatching {
                // Get the LogCollectorTree from Timber forest
                val logCollector = Timber.forest()
                    .find { it is LogCollectorTree } as? LogCollectorTree

                if (logCollector != null) {
                    logCollector.exportLogs(context)
                } else {
                    Timber.w("LogCollectorTree not found - log export unavailable")
                    null
                }
            }.onFailure { e ->
                Timber.e(e, "Failed to export logs")
            }

            onComplete(result)
        }
    }
}

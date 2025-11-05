package com.handcontrol.data.settings

import com.handcontrol.data.database.ConnectionPreference
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import javax.inject.Inject
import javax.inject.Singleton

@Singleton
class SettingsRepository @Inject constructor(
    private val dataStore: SettingsDataStore
) {
    private val _settingsError = MutableStateFlow<String?>(null)
    val settingsError: StateFlow<String?> = _settingsError.asStateFlow()

    val settings: Flow<AppSettings> = dataStore.settings

    suspend fun updateTheme(theme: Theme): Result<Unit> {
        return dataStore.updateTheme(theme).onFailure {
            _settingsError.value = "Failed to save theme setting"
        }
    }

    suspend fun updateRelayFallback(enabled: Boolean): Result<Unit> {
        return dataStore.updateRelayFallback(enabled).onFailure {
            _settingsError.value = "Failed to save relay setting"
        }
    }

    suspend fun updateIpv6Preference(preference: IpPreference): Result<Unit> {
        return dataStore.updateIpv6Preference(preference).onFailure {
            _settingsError.value = "Failed to save IPv6 preference"
        }
    }

    suspend fun updateDirectConnectionTimeout(seconds: Int): Result<Unit> {
        return dataStore.updateDirectConnectionTimeout(seconds).onFailure {
            _settingsError.value = "Failed to save connection timeout"
        }
    }

    suspend fun updateAutoDiscovery(enabled: Boolean): Result<Unit> {
        return dataStore.updateAutoDiscovery(enabled).onFailure {
            _settingsError.value = "Failed to save auto-discovery setting"
        }
    }

    suspend fun updateDiscoveryTimeout(seconds: Int): Result<Unit> {
        return dataStore.updateDiscoveryTimeout(seconds).onFailure {
            _settingsError.value = "Failed to save discovery timeout"
        }
    }

    suspend fun updateLogLevel(level: LogLevel): Result<Unit> {
        return dataStore.updateLogLevel(level).onFailure {
            _settingsError.value = "Failed to save log level"
        }
    }

    suspend fun updateNetworkLogging(enabled: Boolean): Result<Unit> {
        return dataStore.updateNetworkLogging(enabled).onFailure {
            _settingsError.value = "Failed to save network logging setting"
        }
    }

    suspend fun updateNetworkDiagnostics(enabled: Boolean): Result<Unit> {
        return dataStore.updateNetworkDiagnostics(enabled).onFailure {
            _settingsError.value = "Failed to save network diagnostics setting"
        }
    }

    suspend fun updateDefaultConnectionPreference(
        preference: ConnectionPreference
    ): Result<Unit> {
        return dataStore.updateDefaultConnectionPreference(preference).onFailure {
            _settingsError.value = "Failed to save default connection preference"
        }
    }

    suspend fun resetToDefaults(): Result<Unit> {
        return dataStore.resetToDefaults().onFailure {
            _settingsError.value = "Failed to reset settings"
        }
    }

    fun clearError() {
        _settingsError.value = null
    }
}

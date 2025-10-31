package com.handcontrol.data.settings

import android.content.Context
import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.*
import androidx.datastore.preferences.preferencesDataStore
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.catch
import kotlinx.coroutines.flow.map
import timber.log.Timber
import java.io.IOException
import javax.inject.Inject
import javax.inject.Singleton

private val Context.dataStore: DataStore<Preferences> by preferencesDataStore(name = "app_settings")

@Singleton
class SettingsDataStore @Inject constructor(
    private val context: Context
) {
    private object Keys {
        val THEME = stringPreferencesKey("theme")
        val ENABLE_RELAY_FALLBACK = booleanPreferencesKey("enable_relay_fallback")
        val IPV6_PREFERENCE = stringPreferencesKey("ipv6_preference")
        val DIRECT_CONNECTION_TIMEOUT = intPreferencesKey("direct_connection_timeout")
        val AUTO_DISCOVERY = booleanPreferencesKey("auto_discovery")
        val DISCOVERY_TIMEOUT = intPreferencesKey("discovery_timeout")
        val LOG_LEVEL = stringPreferencesKey("log_level")
        val ENABLE_NETWORK_LOGGING = booleanPreferencesKey("enable_network_logging")
        val ENABLE_NETWORK_DIAGNOSTICS = booleanPreferencesKey("enable_network_diagnostics")
    }

    val settings: Flow<AppSettings> = context.dataStore.data
        .catch { exception ->
            if (exception is IOException) {
                Timber.e(exception, "Error reading preferences, emitting defaults")
                emit(emptyPreferences())
            } else {
                throw exception
            }
        }
        .map { prefs ->
            AppSettings(
                theme = Theme.valueOf(prefs[Keys.THEME] ?: Theme.SYSTEM.name),
                enableRelayFallback = prefs[Keys.ENABLE_RELAY_FALLBACK] ?: true,
                ipv6Preference = IpPreference.valueOf(prefs[Keys.IPV6_PREFERENCE] ?: IpPreference.IPV6_PREFERRED.name),
                directConnectionTimeoutSeconds = prefs[Keys.DIRECT_CONNECTION_TIMEOUT] ?: 5,
                autoDiscovery = prefs[Keys.AUTO_DISCOVERY] ?: true,
                discoveryTimeoutSeconds = prefs[Keys.DISCOVERY_TIMEOUT] ?: 5,
                logLevel = LogLevel.valueOf(prefs[Keys.LOG_LEVEL] ?: LogLevel.INFO.name),
                enableNetworkLogging = prefs[Keys.ENABLE_NETWORK_LOGGING] ?: false,
                enableNetworkDiagnostics = prefs[Keys.ENABLE_NETWORK_DIAGNOSTICS] ?: false
            )
        }

    suspend fun updateTheme(theme: Theme): Result<Unit> = runCatching {
        context.dataStore.edit { prefs ->
            prefs[Keys.THEME] = theme.name
        }
        Unit
    }.onFailure { e ->
        Timber.e(e, "Failed to update theme setting")
    }

    suspend fun updateRelayFallback(enabled: Boolean): Result<Unit> = runCatching {
        context.dataStore.edit { prefs ->
            prefs[Keys.ENABLE_RELAY_FALLBACK] = enabled
        }
        Unit
    }.onFailure { e ->
        Timber.e(e, "Failed to update relay fallback setting")
    }

    suspend fun updateIpv6Preference(preference: IpPreference): Result<Unit> = runCatching {
        context.dataStore.edit { prefs ->
            prefs[Keys.IPV6_PREFERENCE] = preference.name
        }
        Unit
    }.onFailure { e ->
        Timber.e(e, "Failed to update IPv6 preference setting")
    }

    suspend fun updateDirectConnectionTimeout(seconds: Int): Result<Unit> = runCatching {
        context.dataStore.edit { prefs ->
            prefs[Keys.DIRECT_CONNECTION_TIMEOUT] = seconds
        }
        Unit
    }.onFailure { e ->
        Timber.e(e, "Failed to update connection timeout setting")
    }

    suspend fun updateAutoDiscovery(enabled: Boolean): Result<Unit> = runCatching {
        context.dataStore.edit { prefs ->
            prefs[Keys.AUTO_DISCOVERY] = enabled
        }
        Unit
    }.onFailure { e ->
        Timber.e(e, "Failed to update auto-discovery setting")
    }

    suspend fun updateDiscoveryTimeout(seconds: Int): Result<Unit> = runCatching {
        context.dataStore.edit { prefs ->
            prefs[Keys.DISCOVERY_TIMEOUT] = seconds
        }
        Unit
    }.onFailure { e ->
        Timber.e(e, "Failed to update discovery timeout setting")
    }

    suspend fun updateLogLevel(level: LogLevel): Result<Unit> = runCatching {
        context.dataStore.edit { prefs ->
            prefs[Keys.LOG_LEVEL] = level.name
        }
        Unit
    }.onFailure { e ->
        Timber.e(e, "Failed to update log level setting")
    }

    suspend fun updateNetworkLogging(enabled: Boolean): Result<Unit> = runCatching {
        context.dataStore.edit { prefs ->
            prefs[Keys.ENABLE_NETWORK_LOGGING] = enabled
        }
        Unit
    }.onFailure { e ->
        Timber.e(e, "Failed to update network logging setting")
    }

    suspend fun updateNetworkDiagnostics(enabled: Boolean): Result<Unit> = runCatching {
        context.dataStore.edit { prefs ->
            prefs[Keys.ENABLE_NETWORK_DIAGNOSTICS] = enabled
        }
        Unit
    }.onFailure { e ->
        Timber.e(e, "Failed to update network diagnostics setting")
    }

    suspend fun resetToDefaults(): Result<Unit> = runCatching {
        context.dataStore.edit { prefs ->
            prefs.clear()
        }
        Unit
    }.onFailure { e ->
        Timber.e(e, "Failed to reset settings to defaults")
    }
}

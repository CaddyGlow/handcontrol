package com.handcontrol.feature.serverdetails

import androidx.lifecycle.SavedStateHandle
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.handcontrol.core.discovery.DiscoveredServer
import com.handcontrol.core.discovery.NsdDiscoveryManager
import com.handcontrol.core.model.ServerDetailInfo
import com.handcontrol.core.model.ServerHealthStatus
import com.handcontrol.core.network.ServerHealthChecker
import com.handcontrol.data.database.ConnectionPreference
import com.handcontrol.data.database.EnrolledServerEntity
import com.handcontrol.data.database.EnrolledServerRepository
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.launch
import timber.log.Timber
import javax.inject.Inject

sealed interface ServerDetailsUiState {
    data object Loading : ServerDetailsUiState
    data class Success(val serverInfo: ServerDetailInfo) : ServerDetailsUiState
    data class Error(val message: String) : ServerDetailsUiState
}

@HiltViewModel
class ServerDetailsViewModel @Inject constructor(
    savedStateHandle: SavedStateHandle,
    private val enrolledServerRepository: EnrolledServerRepository,
    private val serverHealthChecker: ServerHealthChecker,
    private val nsdDiscoveryManager: NsdDiscoveryManager
) : ViewModel() {

    private val serverId: String = checkNotNull(savedStateHandle["serverId"]) {
        "serverId is required"
    }

    private val _healthStatus = MutableStateFlow<ServerHealthStatus>(ServerHealthStatus.Unknown)
    private var healthCheckJob: Job? = null

    val uiState: StateFlow<ServerDetailsUiState> = combine(
        enrolledServerRepository.observeServerById(serverId),
        _healthStatus,
        nsdDiscoveryManager.servers
    ) { server, healthStatus, discoveredServers ->
        if (server == null) {
            ServerDetailsUiState.Error("Server not found")
        } else {
            val discoveredServer = findDiscoveredServer(server, discoveredServers)
            ServerDetailsUiState.Success(
                serverInfo = server.toServerDetailInfo(
                    healthStatus = healthStatus,
                    discoveredServer = discoveredServer
                )
            )
        }
    }.stateIn(
        scope = viewModelScope,
        started = SharingStarted.WhileSubscribed(5000),
        initialValue = ServerDetailsUiState.Loading
    )

    init {
        startPeriodicHealthChecks()
        startMdnsDiscovery()
    }

    private fun startPeriodicHealthChecks() {
        healthCheckJob = viewModelScope.launch {
            while (true) {
                checkHealth()
                delay(15000L) // Check every 15 seconds
            }
        }
    }

    private fun startMdnsDiscovery() {
        viewModelScope.launch {
            try {
                nsdDiscoveryManager.startDiscovery()
            } catch (e: Exception) {
                Timber.e(e, "Failed to start mDNS discovery")
            }
        }
    }

    fun checkHealth() {
        viewModelScope.launch {
            val server = enrolledServerRepository.getServerById(serverId)
            if (server == null) {
                Timber.w("Cannot check health: server $serverId not found")
                return@launch
            }

            _healthStatus.value = ServerHealthStatus.Checking
            Timber.d("Starting health check for server ${server.serverName}")

            val result = serverHealthChecker.checkHealth(server)
            _healthStatus.value = if (result.isReachable) {
                ServerHealthStatus.Connected(result.latencyMs ?: 0)
            } else {
                ServerHealthStatus.Disconnected(result.error)
            }

            Timber.d("Health check completed: ${_healthStatus.value}")
        }
    }

    fun deleteServer() {
        viewModelScope.launch {
            Timber.i("Deleting server: $serverId")
            enrolledServerRepository.removeServer(serverId)
        }
    }

    fun updateConnectionPreference(preference: ConnectionPreference) {
        viewModelScope.launch {
            Timber.i("Updating connection preference for server $serverId to $preference")
            enrolledServerRepository.updateConnectionPreference(serverId, preference)
        }
    }

    override fun onCleared() {
        super.onCleared()
        healthCheckJob?.cancel()
        viewModelScope.launch {
            try {
                nsdDiscoveryManager.stopDiscovery()
            } catch (e: Exception) {
                Timber.e(e, "Failed to stop mDNS discovery")
            }
        }
    }

    private fun findDiscoveredServer(
        server: EnrolledServerEntity,
        discoveredServers: List<DiscoveredServer>
    ): DiscoveredServer? {
        // Try to match by server ID first
        val byServerId = discoveredServers.find { it.serverId == server.serverId }
        if (byServerId != null) return byServerId

        // Try to match by fingerprint
        val byFingerprint = discoveredServers.find { it.fingerprint == server.certFingerprint }
        if (byFingerprint != null) return byFingerprint

        // Try to match by IP and port
        return discoveredServers.find { discovered ->
            val discoveredIps = if (discovered.ips.isNotEmpty()) {
                discovered.ips
            } else {
                listOf(discovered.host)
            }

            discovered.port == server.serverPort &&
                server.ips.any { ip -> discoveredIps.contains(ip) }
        }
    }

    private fun EnrolledServerEntity.toServerDetailInfo(
        healthStatus: ServerHealthStatus,
        discoveredServer: DiscoveredServer?
    ): ServerDetailInfo {
        return ServerDetailInfo(
            serverId = serverId,
            serverName = serverName,
            ips = ips,
            serverPort = serverPort,
            clientId = clientId,
            certFingerprint = certFingerprint,
            enrolledAt = enrolledAt,
            lastConnected = lastConnected,
            lastConnectionMode = lastConnectionMode,
            connectionPreference = connectionPreference,
            relayEnabled = relayEnabled,
            relayUrl = relayUrl,
            healthStatus = healthStatus,
            isDiscoveredViaMdns = discoveredServer != null,
            mdnsServiceName = discoveredServer?.name,
            mdnsLastSeen = if (discoveredServer != null) System.currentTimeMillis() else null
        )
    }
}

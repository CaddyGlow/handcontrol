package com.handcontrol.feature.discovery

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.handcontrol.core.discovery.DiscoveredServer
import com.handcontrol.core.discovery.NsdDiscoveryManager
import com.handcontrol.data.database.EnrolledServerRepository
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import timber.log.Timber
import javax.inject.Inject

data class ServerDiscoveryUiState(
    val servers: List<DiscoveredServer> = emptyList(),
    val enrolledServerIds: Set<String> = emptySet(),
    val isDiscovering: Boolean = false,
    val error: String? = null
)

@HiltViewModel
class ServerDiscoveryViewModel @Inject constructor(
    private val nsdManager: NsdDiscoveryManager,
    private val enrolledServerRepository: EnrolledServerRepository
) : ViewModel() {

    private val _uiState = MutableStateFlow(ServerDiscoveryUiState())
    val uiState: StateFlow<ServerDiscoveryUiState> = _uiState.asStateFlow()

    init {
        observeServers()
        observeEnrolledServers()
    }

    private fun observeServers() {
        viewModelScope.launch {
            nsdManager.servers.collect { servers ->
                _uiState.value = _uiState.value.copy(
                    servers = servers
                )
            }
        }
    }

    private fun observeEnrolledServers() {
        viewModelScope.launch {
            enrolledServerRepository.allServers.collect { servers ->
                _uiState.value = _uiState.value.copy(
                    enrolledServerIds = servers.map { it.serverId }.toSet()
                )
            }
        }
    }

    fun startDiscovery() {
        viewModelScope.launch {
            try {
                _uiState.value = _uiState.value.copy(
                    isDiscovering = true,
                    error = null
                )
                nsdManager.startDiscovery()
                Timber.i("Server discovery started")
            } catch (e: Exception) {
                Timber.e(e, "Failed to start discovery")
                _uiState.value = _uiState.value.copy(
                    isDiscovering = false,
                    error = "Failed to start discovery: ${e.message}"
                )
            }
        }
    }

    fun stopDiscovery() {
        viewModelScope.launch {
            try {
                nsdManager.stopDiscovery()
                _uiState.value = _uiState.value.copy(
                    isDiscovering = false,
                    servers = emptyList()
                )
                Timber.i("Server discovery stopped")
            } catch (e: Exception) {
                Timber.e(e, "Failed to stop discovery")
                _uiState.value = _uiState.value.copy(
                    error = "Failed to stop discovery: ${e.message}"
                )
            }
        }
    }

    fun clearError() {
        _uiState.value = _uiState.value.copy(error = null)
    }

    override fun onCleared() {
        super.onCleared()
        viewModelScope.launch {
            try {
                nsdManager.stopDiscovery()
            } catch (e: Exception) {
                Timber.w(e, "Error stopping discovery on cleanup")
            }
        }
    }
}

package com.handcontrol.feature.serverlist

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.handcontrol.data.database.EnrolledServerEntity
import com.handcontrol.data.database.EnrolledServerRepository
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.launch
import timber.log.Timber
import javax.inject.Inject

sealed interface ServerListUiState {
    data object Loading : ServerListUiState
    data class Success(val servers: List<EnrolledServerEntity>) : ServerListUiState
    data object Empty : ServerListUiState
}

@HiltViewModel
class ServerListViewModel @Inject constructor(
    private val enrolledServerRepository: EnrolledServerRepository
) : ViewModel() {

    val uiState: StateFlow<ServerListUiState> =
        enrolledServerRepository.allServers
            .map { servers ->
                when {
                    servers.isEmpty() -> ServerListUiState.Empty
                    else -> ServerListUiState.Success(servers)
                }
            }
            .stateIn(
                scope = viewModelScope,
                started = SharingStarted.WhileSubscribed(5000),
                initialValue = ServerListUiState.Loading
            )

    fun deleteServer(serverId: String) {
        viewModelScope.launch {
            Timber.i("Deleting server: $serverId")
            enrolledServerRepository.removeServer(serverId)
        }
    }

    fun connectToServer(serverId: String) {
        viewModelScope.launch {
            Timber.i("Navigating to server: $serverId")
            // Note: lastConnected is updated by updateConnectionMode() when connection succeeds
        }
    }
}

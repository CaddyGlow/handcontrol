package com.handcontrol.feature.welcome

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.handcontrol.data.database.EnrolledServerEntity
import com.handcontrol.data.database.EnrolledServerRepository
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.launch
import javax.inject.Inject

@HiltViewModel
class WelcomeViewModel @Inject constructor(
    private val enrolledServerRepository: EnrolledServerRepository
) : ViewModel() {

    val lastConnectedServer: StateFlow<EnrolledServerEntity?> =
        enrolledServerRepository.lastConnectedServer
            .stateIn(
                scope = viewModelScope,
                started = SharingStarted.WhileSubscribed(5000),
                initialValue = null
            )

    fun onServerConnected(serverId: String) {
        viewModelScope.launch {
            enrolledServerRepository.updateLastConnected(serverId)
        }
    }
}

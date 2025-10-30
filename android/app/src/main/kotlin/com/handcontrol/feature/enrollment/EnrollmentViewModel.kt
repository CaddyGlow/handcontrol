package com.handcontrol.feature.enrollment

import android.os.Build
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.handcontrol.data.database.EnrolledServerRepository
import com.handcontrol.data.enrollment.EnrollmentRepository
import com.handcontrol.data.enrollment.EnrollmentResult
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import timber.log.Timber
import javax.inject.Inject

sealed interface EnrollmentUiState {
    data object Idle : EnrollmentUiState
    data object ScanningQr : EnrollmentUiState
    data class QrEnrolling(val token: String) : EnrollmentUiState
    data class ApprovalPending(
        val requestId: String,
        val verificationCode: String,
        val timeoutSeconds: Int
    ) : EnrollmentUiState
    data class Success(val clientId: String, val serverId: String) : EnrollmentUiState
    data class Error(val message: String) : EnrollmentUiState
    data class AlreadyEnrolled(val serverName: String, val serverId: String) : EnrollmentUiState
}

@HiltViewModel
class EnrollmentViewModel @Inject constructor(
    private val enrollmentRepository: EnrollmentRepository,
    private val enrolledServerRepository: EnrolledServerRepository
) : ViewModel() {

    private val _uiState = MutableStateFlow<EnrollmentUiState>(EnrollmentUiState.Idle)
    val uiState: StateFlow<EnrollmentUiState> = _uiState.asStateFlow()

    fun enrollWithQrCode(
        hosts: List<String>,
        port: Int,
        token: String,
        certFingerprint: String,
        serverId: String
    ) {
        viewModelScope.launch {
            try {
                // Check if server is already enrolled (check first host)
                val primaryHost = hosts.firstOrNull() ?: run {
                    _uiState.value = EnrollmentUiState.Error("No valid IP addresses provided")
                    return@launch
                }

                val existingServer = enrolledServerRepository.getServerByHostAndPort(primaryHost, port)
                if (existingServer != null) {
                    Timber.i("Server already enrolled: ${existingServer.serverName}")
                    _uiState.value = EnrollmentUiState.AlreadyEnrolled(existingServer.serverName, existingServer.serverId)
                    return@launch
                }

                _uiState.value = EnrollmentUiState.QrEnrolling(token)
                Timber.i("Enrolling with QR code using ${hosts.size} IP addresses")

                val deviceName = "${Build.MANUFACTURER} ${Build.MODEL}"
                val result = enrollmentRepository.enrollWithToken(
                    hosts = hosts,
                    port = port,
                    token = token,
                    deviceName = deviceName,
                    expectedCertFingerprint = certFingerprint,
                    expectedServerId = serverId
                )

                when (result) {
                    is EnrollmentResult.Success -> {
                        Timber.i("QR enrollment successful")
                        _uiState.value = EnrollmentUiState.Success(result.clientId, result.serverId)
                    }
                    is EnrollmentResult.Error -> {
                        Timber.w("QR enrollment failed: ${result.message}")
                        _uiState.value = EnrollmentUiState.Error(result.message)
                    }
                    else -> {
                        Timber.w("Unexpected enrollment result: $result")
                        _uiState.value = EnrollmentUiState.Error("Unexpected response from server")
                    }
                }
            } catch (e: Exception) {
                Timber.e(e, "QR enrollment exception")
                _uiState.value = EnrollmentUiState.Error("Connection failed: ${e.message}")
            }
        }
    }

    fun requestApprovalPairing(host: String, port: Int, serverId: String?) {
        viewModelScope.launch {
            try {
                // Check if server is already enrolled
                val existingServer = enrolledServerRepository.getServerByHostAndPort(host, port)
                if (existingServer != null) {
                    Timber.i("Server already enrolled: ${existingServer.serverName}")
                    _uiState.value = EnrollmentUiState.AlreadyEnrolled(existingServer.serverName, existingServer.serverId)
                    return@launch
                }

                Timber.i("Requesting approval pairing host=%s port=%d serverId=%s", host, port, serverId)

                val deviceName = "${Build.MANUFACTURER} ${Build.MODEL}"
                val deviceModel = Build.MODEL
                val result = enrollmentRepository.requestApproval(host, port, deviceName, deviceModel, serverId)

                when (result) {
                    is EnrollmentResult.Pending -> {
                        Timber.i("Approval pairing pending: ${result.requestId}")
                        _uiState.value = EnrollmentUiState.ApprovalPending(
                            requestId = result.requestId,
                            verificationCode = result.verificationCode,
                            timeoutSeconds = result.timeoutSeconds
                        )
                        // Start polling for approval status
                        pollApprovalStatus(host, port, result.requestId)
                    }
                    is EnrollmentResult.Success -> {
                        Timber.i("Approval pairing immediately approved")
                        _uiState.value = EnrollmentUiState.Success(result.clientId, result.serverId)
                    }
                    is EnrollmentResult.Error -> {
                        Timber.w("Approval pairing failed: ${result.message}")
                        _uiState.value = EnrollmentUiState.Error(result.message)
                    }
                }
            } catch (e: Exception) {
                Timber.e(e, "Approval pairing exception")
                _uiState.value = EnrollmentUiState.Error("Connection failed: ${e.message}")
            }
        }
    }

    private fun pollApprovalStatus(host: String, port: Int, requestId: String) {
        viewModelScope.launch {
            var attempts = 0
            val maxAttempts = 60 // Poll for up to 60 seconds

            while (attempts < maxAttempts) {
                val currentState = _uiState.value
                if (currentState !is EnrollmentUiState.ApprovalPending) {
                    // User cancelled or state changed
                    break
                }

                delay(1000) // Poll every second
                attempts++

                try {
                    val result = enrollmentRepository.pollApprovalStatus(host, port, requestId)

                    when (result) {
                        is EnrollmentResult.Success -> {
                            Timber.i("Approval pairing approved!")
                            _uiState.value = EnrollmentUiState.Success(result.clientId, result.serverId)
                            break
                        }
                        is EnrollmentResult.Pending -> {
                            // Still pending, continue polling
                            Timber.d("Approval still pending ($attempts/$maxAttempts)")
                        }
                        is EnrollmentResult.Error -> {
                            Timber.w("Approval pairing rejected or timed out")
                            _uiState.value = EnrollmentUiState.Error(result.message)
                            break
                        }
                    }
                } catch (e: Exception) {
                    Timber.e(e, "Error polling approval status")
                    _uiState.value = EnrollmentUiState.Error("Connection lost: ${e.message}")
                    break
                }
            }

            if (attempts >= maxAttempts) {
                Timber.w("Approval polling timeout")
                _uiState.value = EnrollmentUiState.Error("Approval request timed out")
            }
        }
    }

    fun startQrScan() {
        _uiState.value = EnrollmentUiState.ScanningQr
    }

    fun cancelEnrollment() {
        _uiState.value = EnrollmentUiState.Idle
    }

    fun clearError() {
        _uiState.value = EnrollmentUiState.Idle
    }
}

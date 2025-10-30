package com.handcontrol.data.enrollment

import android.content.Context
import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.core.stringPreferencesKey
import androidx.datastore.preferences.preferencesDataStore
import com.google.protobuf.ByteString
import com.handcontrol.core.security.ClientCertificate
import com.handcontrol.core.security.ClientCertificateManager
import com.handcontrol.core.security.VerificationCodeGenerator
import com.handcontrol.grpc.CheckPairingStatusRequest
import com.handcontrol.grpc.EnrollRequest
import com.handcontrol.grpc.PairingStatus
import com.handcontrol.grpc.RemoteControlGrpcKt
import com.handcontrol.grpc.RequestPairingRequest
import dagger.hilt.android.qualifiers.ApplicationContext
import io.grpc.ManagedChannel
import io.grpc.Status
import io.grpc.StatusException
import kotlinx.coroutines.flow.firstOrNull
import kotlinx.coroutines.flow.map
import timber.log.Timber
import javax.inject.Inject
import javax.inject.Singleton

private val Context.enrollmentDataStore: DataStore<Preferences> by preferencesDataStore(
    name = "enrollment_prefs"
)

@Singleton
class GrpcEnrollmentRepository @Inject constructor(
    @ApplicationContext private val context: Context,
    private val certificateManager: ClientCertificateManager,
    private val channelFactory: com.handcontrol.core.network.MtlsGrpcChannelFactory
) : EnrollmentRepository {

    private val CLIENT_ID_KEY = stringPreferencesKey("client_id")

    override suspend fun enrollWithToken(
        host: String,
        port: Int,
        token: String,
        deviceName: String
    ): EnrollmentResult {
        return try {
            val certificate = certificateManager.loadOrCreate()
            val channel = channelFactory.createChannel(host, port)
            val stub = RemoteControlGrpcKt.RemoteControlCoroutineStub(channel)

            val request = EnrollRequest.newBuilder()
                .setEnrollmentToken(token)
                .setClientCertificate(ByteString.copyFrom(certificate.certificateDer))
                .setDeviceName(deviceName)
                .build()

            val response = stub.enroll(request)

            // Extract and pin server certificate for future connections (TOFU)
            val serverCertDer = extractServerCertificate(channel)
            if (serverCertDer != null) {
                val fingerprint = VerificationCodeGenerator.computeFingerprint(serverCertDer)
                certificateManager.pinServerFingerprint(fingerprint)
                Timber.i("Server certificate pinned: $fingerprint")
            } else {
                Timber.w("Could not extract server certificate for pinning")
            }

            channelFactory.shutdownChannel(channel)

            if (response.success) {
                saveClientId(response.clientId)
                Timber.i("QR enrollment successful: clientId=${response.clientId}")
                EnrollmentResult.Success(response.clientId)
            } else {
                Timber.w("QR enrollment failed: ${response.errorMessage}")
                EnrollmentResult.Error(response.errorMessage)
            }
        } catch (e: StatusException) {
            Timber.e(e, "QR enrollment RPC failed")
            EnrollmentResult.Error(mapGrpcError(e.status))
        } catch (e: Exception) {
            Timber.e(e, "QR enrollment failed")
            EnrollmentResult.Error("Network error: ${e.message}")
        }
    }

    override suspend fun requestApproval(
        host: String,
        port: Int,
        deviceName: String,
        deviceModel: String?,
        serverId: String?
    ): EnrollmentResult {
        return try {
            val certificate = certificateManager.loadOrCreate()
            val channel = channelFactory.createChannel(host, port)
            val stub = RemoteControlGrpcKt.RemoteControlCoroutineStub(channel)

            // Make a test RPC to trigger TLS handshake and capture server certificate
            var resolvedServerId: String? = serverId
            try {
                val serverInfo =
                    stub.getServerInfo(com.handcontrol.grpc.ServerInfoRequest.getDefaultInstance())
                Timber.d(
                    "Server info retrieved: serverId=%s hostname=%s version=%s",
                    serverInfo.serverId,
                    serverInfo.hostname,
                    serverInfo.version
                )
                resolvedServerId = serverInfo.serverId.takeIf { it.isNotBlank() } ?: resolvedServerId
            } catch (e: Exception) {
                Timber.w("GetServerInfo request failed: ${e.message}")
            }

            if (resolvedServerId.isNullOrBlank()) {
                Timber.e("Server ID unavailable; cannot compute verification code")
                return EnrollmentResult.Error("Server did not provide an ID for pairing")
            }

            val serverCertDer = extractServerCertificate(channel)
            if (serverCertDer == null) {
                return EnrollmentResult.Error("Failed to extract server certificate")
            }

            val effectiveServerId = resolvedServerId
            val verificationCode = VerificationCodeGenerator.generate(
                certificate.certificateDer,
                serverCertDer,
                effectiveServerId
            )
            Timber.d("Computed verification code=%s serverId=%s", verificationCode, effectiveServerId)

            val request = RequestPairingRequest.newBuilder()
                .setDeviceName(deviceName)
                .setDeviceModel(deviceModel ?: "")
                .setClientCertificate(ByteString.copyFrom(certificate.certificateDer))
                .setVerificationCode(verificationCode)
                .build()

            val response = stub.requestPairing(request)

            if (response.errorMessage.isNotEmpty()) {
                Timber.w("Approval pairing rejected: ${response.errorMessage}")
                return EnrollmentResult.Error(response.errorMessage)
            }

            if (response.verificationCode != verificationCode) {
                Timber.e(
                    "Verification code mismatch - possible MITM attack! expected=%s actual=%s",
                    verificationCode,
                    response.verificationCode
                )
                return EnrollmentResult.Error("Security verification failed")
            }

            val serverFingerprint = VerificationCodeGenerator.computeFingerprint(serverCertDer)
            val responseFingerprint = if (!response.serverCertFingerprint.isEmpty) {
                "SHA256:" + response.serverCertFingerprint.toByteArray()
                    .joinToString("") { "%02x".format(it) }
            } else {
                ""
            }

            if (responseFingerprint.isNotEmpty() && serverFingerprint != responseFingerprint) {
                Timber.e(
                    "Server cert fingerprint mismatch - possible MITM attack! expected=%s actual=%s",
                    serverFingerprint,
                    responseFingerprint
                )
                return EnrollmentResult.Error("Security verification failed")
            }

            // Pin server certificate for future connections (TOFU)
            certificateManager.pinServerFingerprint(serverFingerprint)
            Timber.i("Server certificate pinned: $serverFingerprint")

            if (response.pending) {
                Timber.i("Approval pairing pending: requestId=${response.pairingRequestId}")
                EnrollmentResult.Pending(
                    response.pairingRequestId,
                    response.timeoutSeconds,
                    verificationCode
                )
            } else {
                Timber.w("Approval pairing not pending")
                EnrollmentResult.Error("Unexpected server response")
            }
        } catch (e: StatusException) {
            Timber.e(e, "Approval pairing RPC failed")
            EnrollmentResult.Error(mapGrpcError(e.status))
        } catch (e: Exception) {
            Timber.e(e, "Approval pairing failed")
            EnrollmentResult.Error("Network error: ${e.message}")
        }
    }

    override suspend fun pollApprovalStatus(
        host: String,
        port: Int,
        requestId: String
    ): EnrollmentResult {
        return try {
            val channel = channelFactory.createChannel(host, port)
            val stub = RemoteControlGrpcKt.RemoteControlCoroutineStub(channel)

            val request = CheckPairingStatusRequest.newBuilder()
                .setPairingRequestId(requestId)
                .build()

            val response = stub.checkPairingStatus(request)

            channelFactory.shutdownChannel(channel)

            when (response.status) {
                PairingStatus.PAIRING_STATUS_APPROVED -> {
                    saveClientId(response.clientId)
                    Timber.i("Approval pairing approved: clientId=${response.clientId}")
                    EnrollmentResult.Success(response.clientId)
                }
                PairingStatus.PAIRING_STATUS_PENDING -> {
                    EnrollmentResult.Pending(requestId, 0, "")
                }
                PairingStatus.PAIRING_STATUS_REJECTED -> {
                    Timber.w("Approval pairing rejected by user")
                    EnrollmentResult.Error("Pairing rejected by server")
                }
                PairingStatus.PAIRING_STATUS_TIMEOUT -> {
                    Timber.w("Approval pairing timed out")
                    EnrollmentResult.Error("Pairing request timed out")
                }
                else -> {
                    Timber.w("Unknown pairing status: ${response.status}")
                    EnrollmentResult.Error("Unknown pairing status")
                }
            }
        } catch (e: StatusException) {
            Timber.e(e, "Approval status RPC failed")
            EnrollmentResult.Error(mapGrpcError(e.status))
        } catch (e: Exception) {
            Timber.e(e, "Approval status check failed")
            EnrollmentResult.Error("Network error: ${e.message}")
        }
    }

    override suspend fun activeClientCertificate(): ClientCertificate? {
        val clientId = context.enrollmentDataStore.data
            .map { prefs -> prefs[CLIENT_ID_KEY] }
            .firstOrNull()

        return if (clientId != null) {
            certificateManager.loadOrCreate()
        } else {
            null
        }
    }

    private suspend fun saveClientId(clientId: String) {
        context.enrollmentDataStore.edit { prefs ->
            prefs[CLIENT_ID_KEY] = clientId
        }
    }

    private suspend fun extractServerCertificate(channel: ManagedChannel): ByteArray? {
        // Get the server certificate captured during TLS handshake
        val serverCert = channelFactory.getLastServerCertificate()

        if (serverCert == null) {
            Timber.w("No server certificate captured during TLS handshake")
            return null
        }

        Timber.d("Server certificate extracted: ${serverCert.subjectX500Principal}")
        return serverCert.encoded
    }

    private fun mapGrpcError(status: Status): String {
        return when (status.code) {
            Status.Code.UNAUTHENTICATED -> "Authentication failed"
            Status.Code.PERMISSION_DENIED -> "Permission denied"
            Status.Code.NOT_FOUND -> "Server not found"
            Status.Code.INVALID_ARGUMENT -> "Invalid request"
            Status.Code.DEADLINE_EXCEEDED -> "Request timeout"
            Status.Code.UNAVAILABLE -> "Server unavailable"
            else -> "Network error: ${status.description ?: status.code}"
        }
    }
}

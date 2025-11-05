package com.handcontrol.data.enrollment

import android.content.Context
import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.core.stringPreferencesKey
import androidx.datastore.preferences.preferencesDataStore
import com.google.protobuf.ByteString
import com.handcontrol.core.network.MtlsSslContextFactory
import com.handcontrol.core.network.RelayConnectionException
import com.handcontrol.core.security.ClientCertificate
import com.handcontrol.core.network.relay.RelayGrpcChannelFactory
import com.handcontrol.core.security.ClientCertificateManager
import com.handcontrol.core.security.VerificationCodeGenerator
import com.handcontrol.data.database.ConnectionMode
import com.handcontrol.data.database.ConnectionPreference
import com.handcontrol.data.database.EnrolledServerRepository
import com.handcontrol.grpc.CheckPairingStatusRequest
import com.handcontrol.grpc.EnrollRequest
import com.handcontrol.grpc.PairingStatus
import com.handcontrol.grpc.RemoteControlGrpcKt
import com.handcontrol.grpc.RequestPairingRequest
import com.handcontrol.grpc.ServerInfoRequest
import com.handcontrol.grpc.ServerInfoResponse
import dagger.hilt.android.qualifiers.ApplicationContext
import io.grpc.ManagedChannel
import io.grpc.okhttp.OkHttpChannelBuilder
import io.grpc.Status
import io.grpc.StatusException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.flow.firstOrNull
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.withTimeout
import kotlinx.coroutines.withContext
import timber.log.Timber
import java.io.IOException
import java.security.MessageDigest
import java.security.cert.X509Certificate
import java.time.Instant
import java.util.concurrent.TimeUnit
import java.util.UUID
import javax.inject.Inject
import javax.inject.Singleton

private val Context.enrollmentDataStore: DataStore<Preferences> by preferencesDataStore(
    name = "enrollment_prefs"
)

@Singleton
class GrpcEnrollmentRepository @Inject constructor(
    @ApplicationContext private val context: Context,
    private val certificateManager: ClientCertificateManager,
    private val channelFactory: com.handcontrol.core.network.MtlsGrpcChannelFactory,
    private val relayChannelFactory: RelayGrpcChannelFactory,
    private val enrolledServerRepository: EnrolledServerRepository
) : EnrollmentRepository {

    private val CLIENT_ID_KEY = stringPreferencesKey("client_id")

    private data class EnrollmentChannel(
        val channel: ManagedChannel,
        val serverCertificateProvider: () -> X509Certificate?
    )

    override suspend fun enrollWithToken(
        hosts: List<String>,
        port: Int,
        token: String,
        deviceName: String,
        expectedCertFingerprint: String,
        expectedServerId: String,
        validUntil: Instant?,
        relayOptions: RelayEnrollmentOptions?
    ): EnrollmentResult {
        require(hosts.isNotEmpty()) { "At least one host is required" }
        require(expectedCertFingerprint.isNotEmpty()) { "Certificate fingerprint is required for secure enrollment" }
        require(expectedServerId.isNotEmpty()) { "Server ID is required for secure enrollment" }

        if (validUntil != null && Instant.now().isAfter(validUntil)) {
            Timber.w("Enrollment token expired before attempting enrollment: validUntil=%s", validUntil)
            return EnrollmentResult.Error("This enrollment QR code has expired. Generate a new code and try again.")
        }

        Timber.i("Enrolling with ${hosts.size} IP(s): ${hosts.take(3).joinToString()}")
        Timber.i("Expected cert fingerprint: $expectedCertFingerprint")
        Timber.i("Expected server ID: $expectedServerId")

        // Strategy: IPv6 first -> IPv4 second -> remaining IPs in parallel
        // Server orders IPs as: [best IPv6, best IPv4, other IPv6s, other IPv4s]
        var lastError: EnrollmentResult? = null

        // 1. Try first IP (should be IPv6)
        val firstHost = hosts[0]
        try {
            Timber.d("Trying primary IP (IPv6): $firstHost")
            val result = tryEnrollWithHost(
                host = firstHost,
                port = port,
                token = token,
                deviceName = deviceName,
                timeoutMs = 3000L,
                expectedCertFingerprint = expectedCertFingerprint,
                expectedServerId = expectedServerId,
                validUntil = validUntil,
                allHosts = hosts,
                relayOptions = relayOptions
            )
            if (result is EnrollmentResult.Success) {
                return result
            }
            lastError = result
        } catch (e: Exception) {
            Timber.w(e, "Primary IP $firstHost failed: ${e.javaClass.simpleName}")
        }

        // 2. Try second IP if available (should be IPv4)
        if (hosts.size >= 2) {
            val secondHost = hosts[1]
            try {
                Timber.d("Trying fallback IP (IPv4): $secondHost")
                val result = tryEnrollWithHost(
                    host = secondHost,
                    port = port,
                    token = token,
                    deviceName = deviceName,
                    timeoutMs = 3000L,
                    expectedCertFingerprint = expectedCertFingerprint,
                    expectedServerId = expectedServerId,
                    validUntil = validUntil,
                    allHosts = hosts,
                    relayOptions = relayOptions
                )
                if (result is EnrollmentResult.Success) {
                    return result
                }
                lastError = result
            } catch (e: Exception) {
                Timber.w(e, "Fallback IP $secondHost failed: ${e.javaClass.simpleName}")
            }
        }

        // 3. Try remaining IPs in parallel if any
        val parallelResult = if (hosts.size > 2) {
            val remainingHosts = hosts.drop(2)
            Timber.d("Trying ${remainingHosts.size} remaining IPs in parallel")

            coroutineScope {
                val results = remainingHosts.map { host ->
                    async {
                        try {
                            Timber.d("Trying alternative IP: $host")
                            tryEnrollWithHost(
                                host = host,
                                port = port,
                                token = token,
                                deviceName = deviceName,
                                timeoutMs = 5000L,
                                expectedCertFingerprint = expectedCertFingerprint,
                                expectedServerId = expectedServerId,
                                validUntil = validUntil,
                                allHosts = hosts,
                                relayOptions = relayOptions
                            )
                        } catch (e: Exception) {
                            Timber.w(e, "Alternative IP $host failed: ${e.javaClass.simpleName}")
                            EnrollmentResult.Error("${e.javaClass.simpleName}: ${e.message}")
                        }
                    }
                }

                for (deferred in results) {
                    val result = deferred.await()
                    if (result is EnrollmentResult.Success) {
                        results.forEach { if (it != deferred) it.cancel() }
                        return@coroutineScope result
                    }
                }

                Timber.e("All ${hosts.size} enrollment attempts failed")
                EnrollmentResult.Error("Failed to connect to any of ${hosts.size} IP addresses")
            }
        } else {
            null
        }

        if (parallelResult is EnrollmentResult.Success) {
            return parallelResult
        }
        if (parallelResult is EnrollmentResult.Error) {
            lastError = parallelResult
        }

        val failureResult = lastError ?: EnrollmentResult.Error(
            "Failed to connect to any of ${hosts.size} IP addresses"
        )

        if (relayOptions != null) {
            Timber.i("Direct enrollment attempts failed; attempting relay fallback via ${relayOptions.relayUrl}")
            return tryEnrollViaRelay(
                options = relayOptions,
                token = token,
                deviceName = deviceName,
                expectedCertFingerprint = expectedCertFingerprint,
                expectedServerId = expectedServerId,
                validUntil = validUntil,
                allHosts = hosts,
                port = port
            )
        }

        return failureResult
    }

    private suspend fun tryEnrollWithHost(
        host: String,
        port: Int,
        token: String,
        deviceName: String,
        timeoutMs: Long,
        expectedCertFingerprint: String,
        expectedServerId: String,
        validUntil: Instant?,
        allHosts: List<String>,
        relayOptions: RelayEnrollmentOptions?
    ): EnrollmentResult {
        return withTimeout(timeoutMs) {
            if (validUntil != null && Instant.now().isAfter(validUntil)) {
                Timber.w(
                    "Enrollment token expired during attempt to host=%s port=%d", host, port
                )
                return@withTimeout EnrollmentResult.Error(
                    "This enrollment QR code has expired. Generate a new code and try again."
                )
            }

            var enrollmentChannel: EnrollmentChannel? = null
            try {
                Timber.d(
                    "tryEnrollWithHost: Attempting connection to host=%s port=%d timeout=%dms",
                    host,
                    port,
                    timeoutMs
                )

                val certificate = certificateManager.loadOrCreate()
                enrollmentChannel = openEnrollmentChannel(
                    host = host,
                    port = port,
                    expectedFingerprint = expectedCertFingerprint
                )

                val channel = enrollmentChannel.channel
                Timber.d("tryEnrollWithHost: Unauthenticated gRPC channel created")

                val stub = RemoteControlGrpcKt.RemoteControlCoroutineStub(channel)
                Timber.d("tryEnrollWithHost: Sending enrollment request")

                val request = EnrollRequest.newBuilder()
                    .setEnrollmentToken(token)
                    .setClientCertificate(ByteString.copyFrom(certificate.certificateDer))
                    .setDeviceName(deviceName)
                    .build()

                val response = stub.enroll(request)
                Timber.d("tryEnrollWithHost: Enrollment RPC completed, success=%s", response.success)

                val serverCert = enrollmentChannel.serverCertificateProvider()
                if (serverCert == null) {
                    Timber.e("tryEnrollWithHost: Server certificate not captured during handshake")
                    return@withTimeout EnrollmentResult.Error(
                        "Security verification failed: could not extract server certificate"
                    )
                }

                val serverCertDer = serverCert.encoded
                val fingerprint = VerificationCodeGenerator.computeFingerprint(serverCertDer)
                if (fingerprint != expectedCertFingerprint) {
                    Timber.e(
                        "tryEnrollWithHost: Certificate fingerprint mismatch! expected=%s actual=%s",
                        expectedCertFingerprint,
                        fingerprint
                    )
                    return@withTimeout EnrollmentResult.Error(
                        "Security verification failed: server certificate does not match QR code"
                    )
                }

                certificateManager.pinServerFingerprint(fingerprint)
                Timber.i("Server certificate pinned and verified: %s", fingerprint)

                if (response.success) {
                    return@withTimeout handleSuccessfulEnrollment(
                        channel = channel,
                        response = response,
                        fingerprint = fingerprint,
                        expectedServerId = expectedServerId,
                        allHosts = allHosts,
                        port = port,
                        relayOptions = relayOptions
                    )
                } else {
                    Timber.w("QR enrollment failed: %s", response.errorMessage)
                    EnrollmentResult.Error(response.errorMessage)
                }
            } catch (e: StatusException) {
                val errorMsg = mapGrpcError(e.status)
                Timber.e(
                    e,
                    "QR enrollment RPC failed for host=%s - Status: %s - %s",
                    host,
                    e.status.code,
                    errorMsg
                )
                throw IOException("gRPC ${e.status.code}: $errorMsg", e)
            } catch (e: java.net.UnknownHostException) {
                Timber.e(e, "DNS resolution failed for host=%s", host)
                throw IOException("Cannot resolve host: $host", e)
            } catch (e: java.net.ConnectException) {
                Timber.e(e, "Connection refused for host=%s:%d", host, port)
                throw IOException("Connection refused: $host:$port", e)
            } catch (e: java.net.SocketTimeoutException) {
                Timber.e(e, "Connection timeout for host=%s:%d", host, port)
                throw IOException("Connection timeout: $host:$port", e)
            } catch (e: javax.net.ssl.SSLException) {
                Timber.e(e, "SSL/TLS handshake failed for host=%s", host)
                throw IOException("SSL/TLS error: ${e.message}", e)
            } catch (e: Exception) {
                Timber.e(e, "QR enrollment failed for host=%s - %s", host, e.javaClass.simpleName)
                throw IOException("${e.javaClass.simpleName}: ${e.message}", e)
            } finally {
                enrollmentChannel?.let { shutdownEnrollmentChannel(it.channel) }
            }
        }
    }

    private suspend fun tryEnrollViaRelay(
        options: RelayEnrollmentOptions,
        token: String,
        deviceName: String,
        expectedCertFingerprint: String,
        expectedServerId: String,
        validUntil: Instant?,
        allHosts: List<String>,
        port: Int
    ): EnrollmentResult {
        if (validUntil != null && Instant.now().isAfter(validUntil)) {
            Timber.w(
                "Relay enrollment aborted due to expired token: validUntil=%s",
                validUntil
            )
            return EnrollmentResult.Error(
                "This enrollment QR code has expired. Generate a new code and try again."
            )
        }

        val clientUuid = try {
            UUID.fromString(token)
        } catch (e: IllegalArgumentException) {
            Timber.e(e, "Relay enrollment requires UUID-formatted token")
            return EnrollmentResult.Error(
                "Relay enrollment is unavailable because the enrollment token is not a UUID"
            )
        }

        var channel: ManagedChannel? = null
        return try {
            Timber.d("Attempting relay enrollment via ${options.relayUrl}")

            val certificate = certificateManager.loadOrCreate()
            channel = relayChannelFactory.createChannelViaRelay(
                relayUrl = options.relayUrl,
                serverId = expectedServerId,
                relayToken = options.relayToken,
                clientId = clientUuid.toString(),
                defaultAuthority = computeDefaultRelayAuthority(allHosts, port),
                expectedFingerprint = expectedCertFingerprint,
                allowSelfSignedTls = options.allowSelfSignedTls,
                pinnedCertSha256 = options.pinnedCertSha256
            )

            val stub = RemoteControlGrpcKt.RemoteControlCoroutineStub(channel)
            val request = EnrollRequest.newBuilder()
                .setEnrollmentToken(token)
                .setClientCertificate(ByteString.copyFrom(certificate.certificateDer))
                .setDeviceName(deviceName)
                .build()

            val response = stub.enroll(request)
            Timber.d("Relay enrollment RPC completed, success=%s", response.success)

            val serverCert = relayChannelFactory.getServerCertificate(channel)
            if (serverCert == null) {
                Timber.e("Relay enrollment: server certificate not captured during handshake")
                return EnrollmentResult.Error(
                    "Security verification failed: could not extract server certificate"
                )
            }

            val fingerprint = VerificationCodeGenerator.computeFingerprint(serverCert.encoded)
            if (fingerprint != expectedCertFingerprint) {
                Timber.e(
                    "Relay enrollment fingerprint mismatch! expected=%s actual=%s",
                    expectedCertFingerprint,
                    fingerprint
                )
                return EnrollmentResult.Error(
                    "Security verification failed: server certificate does not match QR code"
                )
            }

            certificateManager.pinServerFingerprint(fingerprint)
            Timber.i("Server certificate verified via relay: %s", fingerprint)

            if (response.success) {
                handleSuccessfulEnrollment(
                    channel = channel,
                    response = response,
                    fingerprint = fingerprint,
                    expectedServerId = expectedServerId,
                    allHosts = allHosts,
                    port = port,
                    relayOptions = options
                )
            } else {
                Timber.w("Relay enrollment failed: ${response.errorMessage}")
                EnrollmentResult.Error(response.errorMessage)
            }
        } catch (e: RelayConnectionException) {
            Timber.e(e, "Relay connection error during enrollment")
            EnrollmentResult.Error("Relay connection failed: ${e.userMessage}")
        } catch (e: StatusException) {
            val errorMsg = mapGrpcError(e.status)
            Timber.e(
                e,
                "Relay enrollment RPC failed - Status: ${e.status.code} - $errorMsg"
            )
            EnrollmentResult.Error("gRPC ${e.status.code}: $errorMsg")
        } catch (e: Exception) {
            Timber.e(e, "Relay enrollment failed - ${e.javaClass.simpleName}")
            EnrollmentResult.Error("Relay enrollment error: ${e.message}")
        } finally {
            if (channel != null) {
                try {
                    relayChannelFactory.shutdownChannel(channel)
                } catch (e: Exception) {
                    Timber.w(e, "Error shutting down relay channel after enrollment attempt")
                }
            }
        }
    }

    private suspend fun handleSuccessfulEnrollment(
        channel: ManagedChannel,
        response: com.handcontrol.grpc.EnrollResponse,
        fingerprint: String,
        expectedServerId: String,
        allHosts: List<String>,
        port: Int,
        relayOptions: RelayEnrollmentOptions?
    ): EnrollmentResult {
        val serverInfo = try {
            getServerInfo(channel)
        } catch (e: Exception) {
            Timber.e(e, "Failed to fetch server information")
            return EnrollmentResult.Error(
                "Failed to fetch server information: ${e.message}"
            )
        }

        val serverId = serverInfo.serverId
        if (serverId.isBlank()) {
            Timber.e("Server returned empty server_id during enrollment")
            return EnrollmentResult.Error("Server returned an invalid server ID")
        }

        if (expectedServerId != serverId) {
            Timber.e(
                "Server ID mismatch! Expected: %s, Got: %s",
                expectedServerId,
                serverId
            )
            return EnrollmentResult.Error(
                "Security verification failed: server ID does not match QR code"
            )
        }

        val relayInfo = if (response.hasRelayInfo()) response.relayInfo else null
        val relayUrl = relayInfo?.relayUrl?.takeIf { it.isNotBlank() } ?: relayOptions?.relayUrl
        val relayToken = relayInfo?.relayToken?.takeIf { it.isNotBlank() } ?: relayOptions?.relayToken
        val relayEnabled = !relayUrl.isNullOrBlank() && !relayToken.isNullOrBlank()

        if (relayEnabled) {
            Timber.i("Relay info registered for server %s: %s", serverId, relayUrl)
        }

        val relayAllowSelfSigned = if (relayEnabled) {
            relayOptions?.allowSelfSignedTls ?: false
        } else {
            false
        }
        val relayPinned = if (relayEnabled) {
            relayOptions?.pinnedCertSha256?.takeIf { !it.isNullOrBlank() }
        } else {
            null
        }

        enrolledServerRepository.saveServer(
            serverId = serverId,
            ips = allHosts,
            serverPort = port,
            clientId = response.clientId,
            serverName = serverInfo.hostname,
            certFingerprint = fingerprint,
            relayEnabled = relayEnabled,
            relayUrl = relayUrl,
            relayToken = relayToken,
            relayAllowSelfSigned = relayAllowSelfSigned,
            relayPinnedCertSha256 = relayPinned,
            initialConnectionMode = ConnectionMode.DIRECT
        )
        Timber.i(
            "Server info saved: serverId=%s, hostname=%s, relay=%s",
            serverId,
            serverInfo.hostname,
            relayEnabled
        )

        val relayRequired =
            (relayInfo?.relayRequired == true) || (relayOptions?.relayRequired == true)
        if (relayEnabled && relayRequired) {
            enrolledServerRepository.updateConnectionPreference(
                serverId,
                ConnectionPreference.RELAY_ONLY
            )
        }

        saveClientId(response.clientId)
        Timber.i(
            "Enrollment successful: clientId=%s, serverId=%s",
            response.clientId,
            serverId
        )

        return EnrollmentResult.Success(response.clientId, serverId)
    }

    private fun computeDefaultRelayAuthority(hosts: List<String>, port: Int): String {
        val host = hosts.firstOrNull()?.trim()?.takeIf { it.isNotEmpty() } ?: "handcontrol.local"
        val authority = if (host.contains(":") && !host.startsWith("[")) {
            "[$host]"
        } else {
            host
        }
        return "$authority:$port"
    }

    override suspend fun requestApproval(
        host: String,
        port: Int,
        deviceName: String,
        deviceModel: String?,
        serverId: String?
    ): EnrollmentResult {
        var enrollmentChannel: EnrollmentChannel? = null
        return try {
            val certificate = certificateManager.loadOrCreate()
            enrollmentChannel = openEnrollmentChannel(
                host = host,
                port = port,
                expectedFingerprint = null
            )
            val channel = enrollmentChannel.channel
            val stub = RemoteControlGrpcKt.RemoteControlCoroutineStub(channel)

            var resolvedServerId: String? = serverId
            try {
                val serverInfo = stub.getServerInfo(ServerInfoRequest.getDefaultInstance())
                Timber.d(
                    "Server info retrieved: serverId=%s hostname=%s version=%s",
                    serverInfo.serverId,
                    serverInfo.hostname,
                    serverInfo.version
                )
                resolvedServerId = serverInfo.serverId.takeIf { it.isNotBlank() } ?: resolvedServerId
            } catch (e: Exception) {
                Timber.w("GetServerInfo request failed: %s", e.message)
            }

            if (resolvedServerId.isNullOrBlank()) {
                Timber.e("Server ID unavailable; cannot compute verification code")
                return EnrollmentResult.Error("Server did not provide an ID for pairing")
            }

            val effectiveServerId = resolvedServerId!!

            val serverCert = enrollmentChannel.serverCertificateProvider()
            if (serverCert == null) {
                Timber.e("Failed to capture server certificate during TLS handshake")
                return EnrollmentResult.Error("Failed to extract server certificate")
            }

            val serverCertDer = serverCert.encoded
            val handshakeFingerprintBytes = computeFingerprintBytes(serverCertDer)

            val pairingRequest = RequestPairingRequest.newBuilder()
                .setDeviceName(deviceName)
                .setDeviceModel(deviceModel ?: "")
                .setClientCertificate(ByteString.copyFrom(certificate.certificateDer))
                .setVerificationCode("")
                .build()

            val response = stub.requestPairing(pairingRequest)

            if (response.errorMessage.isNotEmpty()) {
                Timber.w("Approval pairing rejected: %s", response.errorMessage)
                return EnrollmentResult.Error(response.errorMessage)
            }

            if (response.verificationNonce.isEmpty) {
                Timber.e("Server did not provide verification nonce")
                return EnrollmentResult.Error("Invalid server response: missing nonce")
            }

            val serverFingerprintBytes = response.serverCertFingerprint.toByteArray()
            if (serverFingerprintBytes.isNotEmpty() &&
                !serverFingerprintBytes.contentEquals(handshakeFingerprintBytes)
            ) {
                Timber.e(
                    "Server fingerprint mismatch - expected=%s actual=%s",
                    handshakeFingerprintBytes.joinToString("") { "%02x".format(it) },
                    serverFingerprintBytes.joinToString("") { "%02x".format(it) }
                )
                return EnrollmentResult.Error("Security verification failed")
            }

            val serverFingerprint =
                VerificationCodeGenerator.computeFingerprint(serverCertDer)
            certificateManager.pinServerFingerprint(serverFingerprint)
            Timber.i("Server certificate pinned: %s", serverFingerprint)

            val verificationCode = VerificationCodeGenerator.generate(
                certificate.certificateDer,
                serverCertDer,
                effectiveServerId,
                response.verificationNonce.toByteArray()
            )
            Timber.d(
                "Computed verification code=%s serverId=%s",
                verificationCode,
                effectiveServerId
            )

            if (response.verificationCode.isNotEmpty() &&
                response.verificationCode != verificationCode
            ) {
                Timber.e(
                    "Verification code mismatch - expected=%s actual=%s",
                    verificationCode,
                    response.verificationCode
                )
                return EnrollmentResult.Error("Security verification failed")
            }

            if (response.pending) {
                Timber.i("Approval pairing pending: requestId=%s", response.pairingRequestId)
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
        } finally {
            enrollmentChannel?.let { shutdownEnrollmentChannel(it.channel) }
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

            when (response.status) {
                PairingStatus.PAIRING_STATUS_APPROVED -> {
                    // Fetch server info and save to database
                    val serverId: String
                    try {
                        val serverInfo = getServerInfo(channel)
                        serverId = serverInfo.serverId

                        val serverCertDer = extractServerCertificate(channel)
                        val fingerprint = if (serverCertDer != null) {
                            VerificationCodeGenerator.computeFingerprint(serverCertDer)
                        } else {
                            ""
                        }

                        // Parse relay info if available
                        val relayEnabled = response.hasRelayInfo() && !response.relayInfo.relayUrl.isEmpty()
                        val relayUrl = if (relayEnabled) response.relayInfo.relayUrl else null
                        val relayToken = if (relayEnabled) response.relayInfo.relayToken else null

                        if (relayEnabled) {
                            Timber.i("Relay info received: url=$relayUrl")
                        }

                        val relayAllowSelfSigned = false
                        val relayPinned: String? = null

                        enrolledServerRepository.saveServer(
                            serverId = serverId,
                            ips = listOf(host),
                            serverPort = port,
                            clientId = response.clientId,
                            serverName = serverInfo.hostname,
                            certFingerprint = fingerprint,
                            relayEnabled = relayEnabled,
                            relayUrl = relayUrl,
                            relayToken = relayToken,
                            relayAllowSelfSigned = relayAllowSelfSigned,
                            relayPinnedCertSha256 = relayPinned,
                            initialConnectionMode = ConnectionMode.DIRECT
                        )
                        Timber.i("Server info saved: serverId=$serverId, hostname=${serverInfo.hostname}, relay=${relayEnabled}")

                        if (relayEnabled && response.relayInfo.relayRequired) {
                            enrolledServerRepository.updateConnectionPreference(
                                serverId,
                                ConnectionPreference.RELAY_ONLY
                            )
                        }
                    } catch (e: Exception) {
                        Timber.e(e, "Failed to fetch/save server info")
                        channelFactory.shutdownChannel(channel)
                        return EnrollmentResult.Error("Failed to fetch server information: ${e.message}")
                    }

                    saveClientId(response.clientId)
                    Timber.i("Approval pairing approved: clientId=${response.clientId}, serverId=$serverId")

                    channelFactory.shutdownChannel(channel)
                    EnrollmentResult.Success(response.clientId, serverId)
                }
                PairingStatus.PAIRING_STATUS_PENDING -> {
                    channelFactory.shutdownChannel(channel)
                    EnrollmentResult.Pending(requestId, 0, "")
                }
                PairingStatus.PAIRING_STATUS_REJECTED -> {
                    channelFactory.shutdownChannel(channel)
                    Timber.w("Approval pairing rejected by user")
                    EnrollmentResult.Error("Pairing rejected by server")
                }
                PairingStatus.PAIRING_STATUS_TIMEOUT -> {
                    channelFactory.shutdownChannel(channel)
                    Timber.w("Approval pairing timed out")
                    EnrollmentResult.Error("Pairing request timed out")
                }
                else -> {
                    channelFactory.shutdownChannel(channel)
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
        val serverCert = channelFactory.getServerCertificate(channel)

        if (serverCert == null) {
            Timber.w("No server certificate captured during TLS handshake")
            return null
        }

        Timber.d("Server certificate extracted: ${serverCert.subjectX500Principal}")
        return serverCert.encoded
    }

    private suspend fun getServerInfo(channel: ManagedChannel): ServerInfoResponse {
        val stub = RemoteControlGrpcKt.RemoteControlCoroutineStub(channel)
        return stub.getServerInfo(ServerInfoRequest.getDefaultInstance())
    }

    private suspend fun openEnrollmentChannel(
        host: String,
        port: Int,
        expectedFingerprint: String?
    ): EnrollmentChannel {
        val cleanHost = host.trimStart('[').trimEnd(']')
        val isIpv6 = cleanHost.contains(':')
        val isIpAddress = isIpv6 || cleanHost.matches(Regex("^\\d+\\.\\d+\\.\\d+\\.\\d+$"))

        var capturedCert: X509Certificate? = null
        val sslContext = MtlsSslContextFactory.createSslContext(
            certificateManager = certificateManager,
            expectedFingerprint = expectedFingerprint,
            onServerCertificate = { cert -> capturedCert = cert },
            includeClientCertificate = false
        )

        val builder = OkHttpChannelBuilder
            .forAddress(cleanHost, port)
            .sslSocketFactory(sslContext.socketFactory)
            .hostnameVerifier { _, _ -> true }
            .keepAliveTime(30, TimeUnit.SECONDS)
            .keepAliveTimeout(10, TimeUnit.SECONDS)
            .keepAliveWithoutCalls(true)

        if (isIpAddress) {
            Timber.d("openEnrollmentChannel: IP address detected, overriding authority")
            builder.overrideAuthority("handcontrol.local:$port")
        }

        val channel = builder.build()
        return EnrollmentChannel(channel) { capturedCert }
    }

    private suspend fun shutdownEnrollmentChannel(channel: ManagedChannel) {
        withContext(Dispatchers.IO) {
            try {
                channel.shutdown()
                if (!channel.awaitTermination(2, TimeUnit.SECONDS)) {
                    Timber.d("Enrollment channel did not terminate gracefully, forcing shutdown")
                    channel.shutdownNow()
                    channel.awaitTermination(1, TimeUnit.SECONDS)
                }
            } catch (e: InterruptedException) {
                Timber.w(e, "Enrollment channel shutdown interrupted")
                channel.shutdownNow()
                Thread.currentThread().interrupt()
            } catch (e: Exception) {
                Timber.w(e, "Error shutting down enrollment channel: ${e.message}")
                channel.shutdownNow()
            }
        }
    }

    private fun computeFingerprintBytes(certDer: ByteArray): ByteArray =
        MessageDigest.getInstance("SHA-256").digest(certDer)

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

package com.handcontrol.core.network

import com.handcontrol.core.network.relay.RelayGrpcChannelFactory
import com.handcontrol.data.settings.IpPreference
import com.handcontrol.data.settings.SettingsRepository
import com.handcontrol.data.database.ConnectionMode
import com.handcontrol.data.database.ConnectionPreference
import com.handcontrol.data.database.EnrolledServerEntity
import io.grpc.ConnectivityState
import io.grpc.ManagedChannel
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.TimeoutCancellationException
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeout
import kotlinx.coroutines.withTimeoutOrNull
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.flow.first
import timber.log.Timber
import java.io.IOException
import javax.inject.Inject
import javax.inject.Singleton
import kotlin.coroutines.resume
import java.util.concurrent.TimeUnit

/**
 * Result of a connection attempt
 */
data class ConnectionResult(
    val channel: ManagedChannel,
    val mode: ConnectionMode,
    val connectedAddress: String? = null  // For direct mode, which IP was used
)

/**
 * Manages server connections with automatic fallback from direct to relay
 */
@Singleton
class ServerConnectionManager @Inject constructor(
    private val directChannelFactory: MtlsGrpcChannelFactory,
    private val relayChannelFactory: RelayGrpcChannelFactory,
    private val settingsRepository: SettingsRepository
) {

    private companion object {
        private const val DEFAULT_DIRECT_CONNECT_TIMEOUT_MS = 5000L
        private const val RELAY_CONNECT_TIMEOUT_MS = 15000L
    }

    /**
     * Connects to a server using the best available method.
     *
     * Strategy:
     * 1. Try direct connection to each server IP
     * 2. If all direct connections fail and relay is available, try relay
     * 3. Return the successful connection or throw exception
     *
     * @param server The enrolled server to connect to
     * @param preferenceOverride Optional override for the stored connection preference
     * @return ConnectionResult with the established channel and connection mode
     * @throws Exception if all connection attempts fail
     */
    suspend fun connect(
        server: EnrolledServerEntity,
        preferenceOverride: ConnectionPreference? = null
    ): ConnectionResult = withContext(Dispatchers.IO) {
        Timber.i("Connecting to server ${server.serverName} (${server.serverId})")

        val settings = settingsRepository.settings.first()
        val preference = preferenceOverride ?: server.connectionPreference
        val preferRelay = preference == ConnectionPreference.RELAY_ONLY
        val allowRelayFallback = preference != ConnectionPreference.DIRECT_ONLY
        val directTimeoutMs = TimeUnit.SECONDS.toMillis(
            settings.directConnectionTimeoutSeconds
                .coerceIn(1, 30)
                .toLong()
        ).coerceAtLeast(DEFAULT_DIRECT_CONNECT_TIMEOUT_MS / 5) // safety floor
        val orderedIps = prioritizeIps(server, settings.ipv6Preference)
        Timber.d(
            "Connection preferences -> IPv6=%s, directTimeoutMs=%d, relayFallback=%s, candidateIps=%s",
            settings.ipv6Preference,
            directTimeoutMs,
            settings.enableRelayFallback && allowRelayFallback,
            orderedIps
        )

        // If preferRelay is true and relay is available, skip direct
        if (!preferRelay) {
            // Try direct connections first
            val directResult = tryDirectConnection(server, orderedIps, directTimeoutMs)
            if (directResult != null) {
                Timber.i("Successfully connected via direct mode to ${directResult.connectedAddress}")
                return@withContext directResult
            }

            if (!allowRelayFallback) {
                Timber.i("Connection preference set to direct-only; skipping relay fallback")
                throw Exception("Relay disabled by connection preference")
            }
        }

        // Direct connection failed or was skipped, try relay if available
        if (server.relayEnabled && server.relayUrl != null && server.relayToken != null) {
            if (!settings.enableRelayFallback || !allowRelayFallback) {
                val reason = if (!settings.enableRelayFallback) {
                    "user settings"
                } else {
                    "connection preference"
                }
                Timber.i("Relay fallback disabled by $reason; skipping relay attempt")
                throw Exception("Relay fallback disabled by $reason")
            }

            Timber.i("Direct connection ${if (preferRelay) "skipped" else "failed"}, attempting relay connection")

            try {
                val relayResult = tryRelayConnection(server)
                Timber.i("Successfully connected via relay mode")
                return@withContext relayResult
            } catch (e: RelayConnectionException) {
                Timber.e(e, "Relay connection failed for ${server.serverName}")
                throw e
            }
        }

        // No relay available or relay failed
        if (preferRelay) {
            throw RelayConnectionException("Relay connection not available")
        } else {
            throw Exception("All connection attempts failed")
        }
    }

    /**
     * Attempts direct mTLS connection to the server's IP addresses
     *
     * @return ConnectionResult if successful, null if all attempts fail
     */
    private suspend fun tryDirectConnection(
        server: EnrolledServerEntity,
        ipAddresses: List<String>,
        timeoutMs: Long
    ): ConnectionResult? {

        if (ipAddresses.isEmpty()) {
            Timber.w("No IP addresses available for direct connection")
            return null
        }

        // Try each IP address until one succeeds
        for ((index, ip) in ipAddresses.withIndex()) {
            val channel = try {
                Timber.d("Direct connection attempt ${index + 1}/${ipAddresses.size} to $ip:${server.serverPort}")
                withTimeout(timeoutMs) {
                    directChannelFactory.createChannel(ip, server.serverPort)
                }
            } catch (e: Exception) {
                Timber.d("Direct connection to $ip failed during channel creation: ${e.message}")
                if (index == ipAddresses.size - 1) {
                    Timber.w("All direct connection attempts failed")
                }
                continue
            }

            val ready = try {
                awaitChannelReady(channel, timeoutMs)
            } catch (e: Exception) {
                Timber.d(e, "Direct channel for $ip failed to reach READY state")
                false
            }

            if (ready) {
                Timber.d("Direct channel to $ip reached READY state")
                return ConnectionResult(
                    channel = channel,
                    mode = ConnectionMode.DIRECT,
                    connectedAddress = "$ip:${server.serverPort}"
                )
            } else {
                Timber.d("Direct channel to $ip did not become ready within timeout, closing")
                try {
                    directChannelFactory.forceShutdownChannel(channel)
                } catch (closeError: Exception) {
                    Timber.d(closeError, "Ignored error while force shutting down channel for $ip")
                }

                if (index == ipAddresses.size - 1) {
                    Timber.w("All direct connection attempts failed")
                }
            }
        }

        return null
    }

    private fun prioritizeIps(
        server: EnrolledServerEntity,
        preference: IpPreference
    ): List<String> {
        val rawIps = when {
            server.ips.isNotEmpty() -> server.ips
            server.serverHost != null -> listOf(server.serverHost)
            else -> emptyList()
        }

        if (rawIps.isEmpty()) {
            return emptyList()
        }

        val sanitized = rawIps
            .map { it.trim() }
            .filter { it.isNotEmpty() }
            .distinct()

        val (ipv6, ipv4) = sanitized.partition { it.contains(':') }

        return when (preference) {
            IpPreference.IPV6_PREFERRED -> ipv6 + ipv4
            IpPreference.IPV4_PREFERRED -> ipv4 + ipv6
            IpPreference.IPV6_ONLY -> ipv6
            IpPreference.IPV4_ONLY -> ipv4
        }
    }

    private fun defaultRelayAuthority(server: EnrolledServerEntity): String {
        val preferredHost = server.serverHost?.trim()?.takeIf { it.isNotEmpty() }
        return preferredHost ?: "handcontrol.local:${server.serverPort}"
    }

    /**
     * Attempts connection through relay server
     *
     * @return ConnectionResult if successful
     * @throws RelayConnectionException when the relay connection cannot be established
     */
    private suspend fun tryRelayConnection(server: EnrolledServerEntity): ConnectionResult {
        if (!server.relayEnabled || server.relayUrl == null || server.relayToken == null) {
            throw RelayConnectionException(
                "Relay not configured",
                "The server does not have relay support enabled or is missing relay configuration."
            )
        }

        try {
            Timber.d("Attempting relay connection to ${server.relayUrl}")

            val channel = withTimeout(RELAY_CONNECT_TIMEOUT_MS) {
                relayChannelFactory.createChannelViaRelay(
                    relayUrl = server.relayUrl,
                    serverId = server.serverId,
                    relayToken = server.relayToken,
                    clientId = server.clientId,
                    defaultAuthority = defaultRelayAuthority(server)
                )
            }

            return ConnectionResult(
                channel = channel,
                mode = ConnectionMode.RELAY,
                connectedAddress = null
            )

        } catch (e: TimeoutCancellationException) {
            throw RelayConnectionException(
                "Connection timeout",
                "Failed to establish relay connection within ${RELAY_CONNECT_TIMEOUT_MS / 1000} seconds. The relay server may be unreachable.",
                e
            )
        } catch (e: RelayConnectionException) {
            // Re-throw with more context if needed
            throw e
        } catch (e: IOException) {
            val errorMsg = when {
                e.message?.contains("Software caused connection abort") == true ->
                    "Network connection was interrupted or lost."
                e.message?.contains("Failed to connect") == true ->
                    "Unable to reach the relay server. Check your internet connection."
                e.message?.contains("tunnel health check") == true ->
                    "Relay connection became unresponsive and was closed."
                else -> e.message ?: "Network error occurred during relay connection."
            }
            throw RelayConnectionException("Network error", errorMsg, e)
        } catch (e: Exception) {
            throw RelayConnectionException(
                "Connection failed",
                e.message ?: "An unexpected error occurred: ${e.javaClass.simpleName}",
                e
            )
        }
    }

    /**
     * Shuts down a connection based on its mode
     */
    suspend fun disconnect(result: ConnectionResult) {
        when (result.mode) {
            ConnectionMode.DIRECT -> {
                directChannelFactory.shutdownChannel(result.channel)
            }
            ConnectionMode.RELAY -> {
                relayChannelFactory.shutdownChannel(result.channel)
            }
            ConnectionMode.UNKNOWN -> {
                // Should not happen, but shutdown anyway
                Timber.w("Disconnecting channel with UNKNOWN mode")
                directChannelFactory.shutdownChannel(result.channel)
            }
        }
    }

    /**
     * Shuts down all active connections
     */
    suspend fun shutdown() {
        directChannelFactory.shutdown()
        relayChannelFactory.shutdown()
    }

    private suspend fun awaitChannelReady(
        channel: ManagedChannel,
        timeoutMs: Long
    ): Boolean {
        return withTimeoutOrNull(timeoutMs) {
            var state = channel.getState(true)
            var result: Boolean? = null
            while (result == null) {
                result = when (state) {
                    ConnectivityState.READY -> true
                    ConnectivityState.SHUTDOWN -> false
                    else -> {
                        suspendCancellableCoroutine { cont ->
                            channel.notifyWhenStateChanged(state) {
                                if (!cont.isCompleted) {
                                    cont.resume(Unit)
                                }
                            }
                        }
                        state = channel.getState(true)
                        null
                    }
                }
            }
            result
        } ?: false
    }
}

/**
 * Exception thrown when relay connection fails
 *
 * @param shortMessage Brief error description for logging
 * @param userMessage Detailed, user-friendly error message
 * @param cause The underlying cause of the error
 */
class RelayConnectionException(
    val shortMessage: String,
    val userMessage: String = shortMessage,
    cause: Throwable? = null
) : Exception("$shortMessage: $userMessage", cause) {
    // Convenience constructor for backwards compatibility
    constructor(message: String, cause: Throwable? = null) : this(message, message, cause)
}

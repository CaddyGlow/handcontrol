package com.handcontrol.core.network

import com.handcontrol.core.network.relay.RelayGrpcChannelFactory
import com.handcontrol.data.database.ConnectionMode
import com.handcontrol.data.database.EnrolledServerEntity
import io.grpc.ManagedChannel
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeout
import timber.log.Timber
import javax.inject.Inject
import javax.inject.Singleton

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
    private val relayChannelFactory: RelayGrpcChannelFactory
) {

    /**
     * Connects to a server using the best available method
     *
     * Strategy:
     * 1. Try direct connection to each server IP
     * 2. If all direct connections fail and relay is available, try relay
     * 3. Return the successful connection or throw exception
     *
     * @param server The enrolled server to connect to
     * @param preferRelay If true, skip direct and go straight to relay
     * @return ConnectionResult with the established channel and connection mode
     * @throws Exception if all connection attempts fail
     */
    suspend fun connect(
        server: EnrolledServerEntity,
        preferRelay: Boolean = false
    ): ConnectionResult = withContext(Dispatchers.IO) {
        Timber.i("Connecting to server ${server.serverName} (${server.serverId})")

        // If preferRelay is true and relay is available, skip direct
        if (!preferRelay) {
            // Try direct connections first
            val directResult = tryDirectConnection(server)
            if (directResult != null) {
                Timber.i("Successfully connected via direct mode to ${directResult.connectedAddress}")
                return@withContext directResult
            }
        }

        // Direct connection failed or was skipped, try relay if available
        if (server.relayEnabled && server.relayUrl != null && server.relayToken != null) {
            Timber.i("Direct connection ${if (preferRelay) "skipped" else "failed"}, attempting relay connection")

            val relayResult = tryRelayConnection(server)
            if (relayResult != null) {
                Timber.i("Successfully connected via relay mode")
                return@withContext relayResult
            }

            throw Exception("Relay connection failed")
        }

        // No relay available or relay failed
        throw Exception(if (preferRelay) "Relay connection not available" else "All connection attempts failed")
    }

    /**
     * Attempts direct mTLS connection to the server's IP addresses
     *
     * @return ConnectionResult if successful, null if all attempts fail
     */
    private suspend fun tryDirectConnection(server: EnrolledServerEntity): ConnectionResult? {
        val ipAddresses = if (server.ips.isNotEmpty()) {
            server.ips
        } else {
            // Fallback to serverHost for backward compatibility
            server.serverHost?.let { listOf(it) } ?: emptyList()
        }

        if (ipAddresses.isEmpty()) {
            Timber.w("No IP addresses available for direct connection")
            return null
        }

        // Try each IP address until one succeeds
        for ((index, ip) in ipAddresses.withIndex()) {
            try {
                Timber.d("Direct connection attempt ${index + 1}/${ipAddresses.size} to $ip:${server.serverPort}")

                val channel = withTimeout(5000L) {
                    directChannelFactory.createChannel(ip, server.serverPort)
                }

                // Connection successful
                return ConnectionResult(
                    channel = channel,
                    mode = ConnectionMode.DIRECT,
                    connectedAddress = "$ip:${server.serverPort}"
                )

            } catch (e: Exception) {
                Timber.d("Direct connection to $ip failed: ${e.message}")

                if (index == ipAddresses.size - 1) {
                    Timber.w("All direct connection attempts failed")
                }
                // Try next IP
            }
        }

        return null
    }

    /**
     * Attempts connection through relay server
     *
     * @return ConnectionResult if successful, null if relay connection fails
     */
    private suspend fun tryRelayConnection(server: EnrolledServerEntity): ConnectionResult? {
        if (!server.relayEnabled || server.relayUrl == null || server.relayToken == null) {
            Timber.w("Relay not properly configured for server ${server.serverId}")
            return null
        }

        return try {
            Timber.d("Attempting relay connection to ${server.relayUrl}")

            val channel = withTimeout(10000L) {
                relayChannelFactory.createChannelViaRelay(
                    relayUrl = server.relayUrl,
                    serverId = server.serverId,
                    relayToken = server.relayToken,
                    clientId = server.clientId
                )
            }

            ConnectionResult(
                channel = channel,
                mode = ConnectionMode.RELAY,
                connectedAddress = null
            )

        } catch (e: Exception) {
            Timber.e(e, "Relay connection failed")
            null
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
}

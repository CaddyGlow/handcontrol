package com.handcontrol.core.network.relay

import com.handcontrol.core.network.MtlsSslContextFactory
import com.handcontrol.core.network.RelayConnectionException
import com.handcontrol.core.security.ClientCertificateManager
import io.grpc.Attributes
import io.grpc.EquivalentAddressGroup
import io.grpc.ManagedChannel
import io.grpc.NameResolver
import io.grpc.NameResolver.ResolutionResult
import io.grpc.Status
import io.grpc.okhttp.OkHttpChannelBuilder
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import timber.log.Timber
import java.io.IOException
import java.net.InetAddress
import java.net.InetSocketAddress
import java.net.ServerSocket
import java.net.Socket
import java.net.SocketAddress
import java.net.SocketException
import java.net.URI
import java.util.concurrent.TimeUnit
import javax.inject.Inject
import javax.inject.Singleton

/**
 * Creates gRPC channels over relay tunnels by establishing a local TCP bridge
 * that forwards traffic through the WebSocket tunnel to the relay server.
 */
@Singleton
class RelayGrpcChannelFactory @Inject constructor(
    private val tunnelFactory: RelayTunnelFactory,
    private val certificateManager: ClientCertificateManager
) {
    private val scope = CoroutineScope(Dispatchers.IO)
    private val activeBridges = mutableMapOf<ManagedChannel, BridgeContext>()

    /**
     * Creates a gRPC channel that routes through a relay tunnel
     *
     * @param relayUrl The relay server URL
     * @param serverId The target server ID
     * @param relayToken JWT token for relay authentication
     * @param clientId The client ID
     * @return ManagedChannel configured to use the relay tunnel
     */
    suspend fun createChannelViaRelay(
        relayUrl: String,
        serverId: String,
        relayToken: String,
        clientId: String,
        defaultAuthority: String,
        expectedFingerprint: String? = null
    ): ManagedChannel = withContext(Dispatchers.IO) {
        Timber.i("Creating gRPC channel via relay for server $serverId")

        // Open relay tunnel
        val tunnel = tunnelFactory.openTunnel(
            relayUrl = relayUrl,
            serverId = serverId,
            relayToken = relayToken,
            clientId = clientId,
            clientVersion = "0.1.0"
        )

        Timber.d(
            "Relay tunnel established, tunnel_id=${tunnel.tunnelId}, relay_host=${tunnel.relayHost}, server_authority=${tunnel.serverAuthority}"
        )

        // Create local TCP bridge
        val bridge = startLocalBridge(tunnel)

        Timber.d("Local bridge started on port ${bridge.localPort}")

        val authority = tunnel.serverAuthority ?: defaultAuthority
        if (tunnel.serverAuthority == null) {
            Timber.w(
                "Relay connect_ack omitted server authority; falling back to default '$defaultAuthority'"
            )
        }

        val authorityHost = parseAuthorityHost(authority)
        if (authorityHost == null) {
            Timber.e("Unable to determine TLS authority host from '$authority'")
            bridge.shutdown()
            throw RelayConnectionException(
                "Invalid relay authority",
                "The relay returned an invalid TLS authority."
            )
        }

        Timber.d("Relay authority host resolved to '$authorityHost'; installing loopback resolver")

        val channel = createMtlsChannel(
            authority = authority,
            loopbackPort = bridge.localPort,
            expectedFingerprint = expectedFingerprint
        )

        activeBridges[channel] = bridge
        Timber.i("Relay gRPC channel created successfully")
        channel
    }

    /**
     * Shuts down a relay channel and its associated bridge
     */
    suspend fun shutdownChannel(channel: ManagedChannel) = withContext(Dispatchers.IO) {
        Timber.i("Shutting down relay gRPC channel")

        val bridge = activeBridges.remove(channel)
        if (bridge != null) {
            bridge.shutdown()
        }

        channel.shutdown()
        try {
            if (!channel.awaitTermination(5, TimeUnit.SECONDS)) {
                Timber.w("Channel did not terminate gracefully, forcing shutdown")
                channel.shutdownNow()
                channel.awaitTermination(2, TimeUnit.SECONDS)
            }
        } catch (e: InterruptedException) {
            Timber.e(e, "Interrupted while shutting down channel")
            channel.shutdownNow()
            Thread.currentThread().interrupt()
        }
    }

    /**
     * Starts a local TCP server that bridges to the relay tunnel
     */
    private suspend fun startLocalBridge(tunnel: RelayTunnel): BridgeContext {
        // Create server socket on random available port
        val serverSocket = ServerSocket(0)
        val localPort = serverSocket.localPort

        Timber.d("Starting local bridge on port $localPort")

        // Accept connections in background and bridge to tunnel
        val acceptJob = scope.launch {
            try {
                while (true) {
                    val socket = serverSocket.accept()
                    Timber.d("Accepted local connection from ${socket.remoteSocketAddress}")

                    // Bridge this socket to the tunnel
                    launch {
                        bridgeSocketToTunnel(socket, tunnel)
                    }
                }
            } catch (e: IOException) {
                if (!serverSocket.isClosed) {
                    Timber.e(e, "Error accepting connections on local bridge")
                }
            }
        }

        return BridgeContext(
            serverSocket = serverSocket,
            localPort = localPort,
            acceptJob = acceptJob,
            tunnel = tunnel
        )
    }

    /**
     * Bridges a local TCP socket to a relay tunnel with bidirectional forwarding
     */
    private suspend fun bridgeSocketToTunnel(socket: Socket, tunnel: RelayTunnel) {
        Timber.d("Starting bidirectional bridge for socket ${socket.remoteSocketAddress}")

        // Check tunnel health before starting
        if (!tunnel.isHealthy()) {
            Timber.e("Refusing to bridge socket - tunnel is unhealthy")
            try {
                socket.close()
            } catch (e: IOException) {
                Timber.w(e, "Error closing socket for unhealthy tunnel")
            }
            return
        }

        try {
            val inputStream = socket.getInputStream()
            val outputStream = socket.getOutputStream()

            // Launch coroutine to forward socket → tunnel
            val sendJob = scope.launch {
                try {
                    val buffer = ByteArray(8192)
                    while (true) {
                        val bytesRead = inputStream.read(buffer)
                        if (bytesRead == -1) {
                            Timber.d("Socket input stream closed")
                            break
                        }

                        // Check tunnel health before sending
                        if (!tunnel.isHealthy()) {
                            Timber.w("Tunnel became unhealthy, stopping socket → tunnel forwarding")
                            break
                        }

                        val data = buffer.copyOfRange(0, bytesRead)
                        tunnel.sendData(data)
                    }
                } catch (e: IOException) {
                    when {
                        e.isExpectedSocketClosure() -> {
                            Timber.d("Socket read completed: ${e.message ?: "closed"}")
                        }
                        tunnel.isHealthy() -> {
                            Timber.d("Socket read error: ${e.message}")
                        }
                        else -> {
                            Timber.w("Socket read error (tunnel unhealthy): ${e.message}")
                        }
                    }
                } catch (e: Exception) {
                    Timber.e(e, "Error forwarding socket → tunnel")
                }
            }

            // Launch coroutine to forward tunnel → socket
            val receiveJob = scope.launch {
                try {
                    for (data in tunnel.incomingData) {
                        outputStream.write(data)
                        outputStream.flush()
                    }
                    Timber.d("Tunnel incoming channel closed")
                } catch (e: IOException) {
                    when {
                        e.isExpectedSocketClosure() -> {
                            Timber.d("Socket write completed: ${e.message ?: "closed"}")
                        }
                        tunnel.isHealthy() -> {
                            Timber.d("Socket write error: ${e.message}")
                        }
                        else -> {
                            Timber.w("Socket write error (tunnel unhealthy): ${e.message}")
                        }
                    }
                } catch (e: Exception) {
                    Timber.e(e, "Error forwarding tunnel → socket")
                }
            }

            // Wait for both jobs to complete
            try {
                sendJob.join()
                receiveJob.join()
            } catch (_: CancellationException) {
                Timber.d("Socket bridge cancelled for ${socket.remoteSocketAddress}")
            }
        } catch (_: CancellationException) {
            Timber.d("Socket bridge cancelled for ${socket.remoteSocketAddress}")
        } catch (e: IOException) {
            if (!e.isExpectedSocketClosure()) {
                Timber.e(e, "Error in socket bridge")
            } else {
                Timber.d("Socket bridge closed: ${e.message ?: "socket closed"}")
            }
        } catch (e: Exception) {
            Timber.e(e, "Error in socket bridge")
        } finally {
            try {
                socket.close()
            } catch (e: IOException) {
                Timber.w(e, "Error closing bridged socket")
            }
        }
    }

    /**
     * Creates an mTLS-enabled gRPC channel to a specific host/port with authority override
     */
    private suspend fun createMtlsChannel(
        authority: String,
        loopbackPort: Int,
        expectedFingerprint: String?
    ): ManagedChannel {
        val sslContext = MtlsSslContextFactory.createSslContext(
            certificateManager = certificateManager,
            expectedFingerprint = expectedFingerprint
        )

        val builder = OkHttpChannelBuilder
            .forTarget("loopback:///$authority")
            .nameResolverFactory(LoopbackNameResolverFactory(loopbackPort))
            .sslSocketFactory(sslContext.socketFactory)
            .hostnameVerifier { _, _ -> true }
            .overrideAuthority(authority)
            .keepAliveTime(30, TimeUnit.SECONDS)
            .keepAliveTimeout(10, TimeUnit.SECONDS)
            .keepAliveWithoutCalls(true)
        return builder.build()
    }

    suspend fun shutdown() = withContext(Dispatchers.IO) {
        Timber.i("Shutting down all relay channels")
        activeBridges.keys.toList().forEach { channel ->
            shutdownChannel(channel)
        }
    }
}

private fun parseAuthorityHost(authority: String): String? {
    if (authority.isBlank()) {
        return null
    }

    return if (authority.startsWith("[")) {
        authority.substringAfter('[').substringBefore(']').takeIf { it.isNotBlank() }
    } else {
        authority.substringBefore(':').ifEmpty { authority }
    }
}

private class LoopbackNameResolverFactory(
    private val loopbackPort: Int
) : NameResolver.Factory() {
    override fun newNameResolver(targetUri: URI, args: NameResolver.Args): NameResolver? {
        return if (targetUri.scheme == getDefaultScheme()) {
            LoopbackNameResolver(loopbackPort)
        } else {
            null
        }
    }

    override fun getDefaultScheme(): String = "loopback"
}

private class LoopbackNameResolver(
    private val loopbackPort: Int
) : NameResolver() {
    override fun getServiceAuthority(): String = "loopback"

    override fun start(listener: Listener2) {
        val addresses = buildList<SocketAddress> {
            runCatching { InetAddress.getByName("127.0.0.1") }
                .onSuccess { add(InetSocketAddress(it, loopbackPort)) }
                .onFailure { Timber.e(it, "Failed to resolve IPv4 loopback address") }

            runCatching { InetAddress.getByName("::1") }
                .onSuccess { add(InetSocketAddress(it, loopbackPort)) }
                .onFailure { Timber.d("IPv6 loopback not available: ${it.message}") }
        }

        if (addresses.isEmpty()) {
            listener.onError(Status.UNAVAILABLE.withDescription("Loopback addresses unavailable"))
            return
        }
        val addressGroup = EquivalentAddressGroup(addresses)
        val result = ResolutionResult.newBuilder()
            .setAddresses(listOf(addressGroup))
            .setAttributes(Attributes.EMPTY)
            .build()
        listener.onResult(result)
    }

    override fun refresh() {
        // No dynamic data to refresh; resolution is static.
    }

    override fun shutdown() {
        // Nothing to release.
    }
}

private fun IOException.isExpectedSocketClosure(): Boolean {
    val messageText = message?.lowercase() ?: return false
    return messageText.contains("socket closed") ||
        messageText.contains("software caused connection abort") ||
        messageText.contains("connection reset") ||
        messageText.contains("broken pipe")
}

/**
 * Context for a local TCP bridge to a relay tunnel
 */
private data class BridgeContext(
    val serverSocket: ServerSocket,
    val localPort: Int,
    val acceptJob: Job,
    val tunnel: RelayTunnel
) {
    suspend fun shutdown() {
        Timber.d("Shutting down local bridge on port $localPort")

        acceptJob.cancelAndJoin()

        try {
            serverSocket.close()
        } catch (e: IOException) {
            Timber.w(e, "Error closing server socket")
        }

        tunnel.close()
    }
}

package com.handcontrol.core.network.relay

import com.handcontrol.core.security.ClientCertificateManager
import com.handcontrol.core.security.VerificationCodeGenerator
import io.grpc.ManagedChannel
import io.grpc.okhttp.OkHttpChannelBuilder
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import timber.log.Timber
import java.io.IOException
import java.net.ServerSocket
import java.net.Socket
import java.security.KeyStore
import java.util.concurrent.TimeUnit
import javax.inject.Inject
import javax.inject.Singleton
import javax.net.ssl.KeyManagerFactory
import javax.net.ssl.SSLContext
import javax.net.ssl.X509TrustManager

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
        defaultAuthority: String
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

        // Create mTLS gRPC channel to localhost (which forwards to relay)
        val channel = createMtlsChannel(
            host = "localhost",
            port = bridge.localPort,
            authority = authority
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
                    if (tunnel.isHealthy()) {
                        Timber.d("Socket read error: ${e.message}")
                    } else {
                        Timber.w("Socket read error (tunnel unhealthy): ${e.message}")
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
                    if (tunnel.isHealthy()) {
                        Timber.d("Socket write error: ${e.message}")
                    } else {
                        Timber.w("Socket write error (tunnel unhealthy): ${e.message}")
                    }
                } catch (e: Exception) {
                    Timber.e(e, "Error forwarding tunnel → socket")
                }
            }

            // Wait for both jobs to complete
            sendJob.join()
            receiveJob.join()

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
        host: String,
        port: Int,
        authority: String
    ): ManagedChannel {
        val clientCert = certificateManager.loadOrCreate()
        val pinnedFingerprint = certificateManager.getPinnedServerFingerprint()
        if (pinnedFingerprint == null) {
            Timber.w("No pinned server fingerprint available; relay connection will trust first certificate")
        }
        val sslContext = createMtlsSslContext(pinnedFingerprint)

        return OkHttpChannelBuilder
            .forAddress(host, port)
            .sslSocketFactory(sslContext.socketFactory)
            .hostnameVerifier { _, _ -> true }
            .overrideAuthority(authority)
            .keepAliveTime(30, TimeUnit.SECONDS)
            .keepAliveTimeout(10, TimeUnit.SECONDS)
            .keepAliveWithoutCalls(true)
            .build()
    }

    /**
     * Creates SSL context for mTLS
     */
    private suspend fun createMtlsSslContext(
        pinnedFingerprint: String?
    ): SSLContext {
        // Load Android Keystore
        val androidKeyStore = KeyStore.getInstance("AndroidKeyStore").apply {
            load(null)
        }

        // Create key manager with client certificate
        val keyManagerFactory = KeyManagerFactory.getInstance(
            KeyManagerFactory.getDefaultAlgorithm()
        )
        keyManagerFactory.init(androidKeyStore, null)

        // Create trust manager that enforces the pinned fingerprint when available
        val trustManager = object : X509TrustManager {
            override fun checkClientTrusted(
                chain: Array<out java.security.cert.X509Certificate>?,
                authType: String?
            ) {
                // Not used on client side
            }

            override fun checkServerTrusted(
                chain: Array<out java.security.cert.X509Certificate>?,
                authType: String?
            ) {
                if (chain == null || chain.isEmpty()) {
                    throw javax.net.ssl.SSLException("Server certificate chain is empty")
                }

                val serverCert = chain[0]

                if (pinnedFingerprint != null) {
                    val currentFingerprint = VerificationCodeGenerator.computeFingerprint(
                        serverCert.encoded
                    )

                    if (currentFingerprint != pinnedFingerprint) {
                        Timber.e("Relay TLS fingerprint mismatch: expected=$pinnedFingerprint actual=$currentFingerprint")
                        throw javax.net.ssl.SSLException("Server certificate fingerprint does not match pinned value")
                    }

                    Timber.d("Relay TLS fingerprint verified via pinned certificate")
                } else {
                    Timber.w("Relay TLS connection without pinned fingerprint; treating certificate as first trust")
                }

                try {
                    serverCert.checkValidity()
                } catch (e: Exception) {
                    throw javax.net.ssl.SSLException("Server certificate is not valid", e)
                }
            }

            override fun getAcceptedIssuers(): Array<java.security.cert.X509Certificate> {
                return arrayOf()
            }
        }

        // Create SSL context
        val sslContext = SSLContext.getInstance("TLS")
        sslContext.init(
            keyManagerFactory.keyManagers,
            arrayOf(trustManager),
            null
        )

        return sslContext
    }

    suspend fun shutdown() = withContext(Dispatchers.IO) {
        Timber.i("Shutting down all relay channels")
        activeBridges.keys.toList().forEach { channel ->
            shutdownChannel(channel)
        }
    }
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

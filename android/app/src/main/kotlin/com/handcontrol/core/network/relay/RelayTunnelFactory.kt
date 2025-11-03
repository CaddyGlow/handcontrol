package com.handcontrol.core.network.relay

import com.handcontrol.core.network.RelayConnectionException
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import okio.ByteString
import timber.log.Timber
import java.io.IOException
import java.security.cert.X509Certificate
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicLong
import javax.inject.Inject
import javax.inject.Singleton
import javax.net.ssl.SSLContext
import javax.net.ssl.TrustManager
import javax.net.ssl.X509TrustManager

/**
 * Message sent by client to initiate tunnel connection
 */
@Serializable
data class ConnectMessage(
    val type: String = "connect",
    val server_id: String,
    val relay_token: String,
    val client_id: String,
    val client_version: String
)

/**
 * Acknowledgement from relay after connect
 */
@Serializable
data class ConnectAck(
    val type: String,
    val status: String,
    val tunnel_id: String? = null,
    val relay_host: String? = null,
    val expires_at: Long? = null,
    val server_authority: String? = null
)

/**
 * Message sent to signal tunnel is ready for data
 */
@Serializable
data class TunnelReadyMessage(
    val type: String = "tunnel_ready",
    val tunnel_id: String,
    val role: String = "client"
)

/**
 * Represents an active relay tunnel with bidirectional data channels
 */
data class RelayTunnel(
    val tunnelId: String,
    val webSocket: WebSocket,
    val incomingData: Channel<ByteArray>,
    val relayHost: String?,
    val serverAuthority: String?,
    val healthMonitor: TunnelHealthMonitor
) {
    suspend fun sendData(data: ByteArray) {
        healthMonitor.recordActivity()
        webSocket.send(ByteString.of(*data))
    }

    fun close() {
        healthMonitor.stop()
        webSocket.close(1000, "Tunnel closed")
        incomingData.close()
    }

    fun isHealthy(): Boolean = healthMonitor.isHealthy()
}

/**
 * Monitors tunnel health by tracking message activity and detecting stalls
 */
class TunnelHealthMonitor(
    private val scope: CoroutineScope,
    private val onUnhealthy: () -> Unit
) {
    private val lastActivityTime = AtomicLong(System.currentTimeMillis())
    private val isRunning = AtomicBoolean(true)
    private var monitorJob: Job? = null

    companion object {
        private const val HEALTH_CHECK_INTERVAL_MS = 10000L // Check every 10 seconds
        private const val MAX_IDLE_TIME_MS = 120000L // 120 seconds without any activity (2 minutes)
        // Note: OkHttp sends automatic pings every 20s, but those don't trigger recordActivity()
        // Server also sends pings every 20s which DO trigger recordActivity()
        // So we should see activity at least every 20-40s from pong responses
    }

    fun start() {
        monitorJob = scope.launch {
            while (isActive && isRunning.get()) {
                delay(HEALTH_CHECK_INTERVAL_MS)

                val idleTime = System.currentTimeMillis() - lastActivityTime.get()
                Timber.v("Tunnel health check: idle for ${idleTime}ms (max: ${MAX_IDLE_TIME_MS}ms)")

                if (idleTime > MAX_IDLE_TIME_MS) {
                    Timber.w("Tunnel health check failed: no activity for ${idleTime}ms (max: ${MAX_IDLE_TIME_MS}ms)")
                    isRunning.set(false)
                    onUnhealthy()
                    break
                }
            }
        }
    }

    fun recordActivity() {
        lastActivityTime.set(System.currentTimeMillis())
    }

    fun isHealthy(): Boolean = isRunning.get()

    fun stop() {
        isRunning.set(false)
        monitorJob?.cancel()
    }
}

/**
 * Factory for creating WebSocket-based relay tunnels
 */
@Singleton
class RelayTunnelFactory @Inject constructor() {

    private val scope = CoroutineScope(Dispatchers.IO)

    // encodeDefaults ensures we transmit required control fields like "type" even when defaults are used
    private val json = Json {
        ignoreUnknownKeys = true
        encodeDefaults = true
    }

    private val okHttpClient by lazy {
        createOkHttpClient()
    }

    private fun createOkHttpClient(): OkHttpClient {
        // Create a trust manager that accepts all certificates (for self-signed relay certs)
        val trustAllCerts = arrayOf<TrustManager>(object : X509TrustManager {
            override fun checkClientTrusted(chain: Array<out X509Certificate>?, authType: String?) {}
            override fun checkServerTrusted(chain: Array<out X509Certificate>?, authType: String?) {}
            override fun getAcceptedIssuers(): Array<X509Certificate> = arrayOf()
        })

        val sslContext = SSLContext.getInstance("TLS")
        sslContext.init(null, trustAllCerts, java.security.SecureRandom())

        return OkHttpClient.Builder()
            .connectTimeout(10, TimeUnit.SECONDS)
            .readTimeout(0, TimeUnit.SECONDS) // No read timeout for persistent connection
            .writeTimeout(10, TimeUnit.SECONDS)
            .pingInterval(20, TimeUnit.SECONDS) // WebSocket ping every 20 seconds
            .sslSocketFactory(sslContext.socketFactory, trustAllCerts[0] as X509TrustManager)
            .hostnameVerifier { _, _ -> true } // Accept all hostnames for self-signed certs
            .build()
    }

    /**
     * Opens a relay tunnel to the specified server
     *
     * @param relayUrl The relay server URL (e.g., "wss://relay.example.com")
     * @param serverId The server ID to connect to
     * @param relayToken JWT token for authentication
     * @param clientId The client ID
     * @param clientVersion The client version string
     * @return RelayTunnel on success
     */
    suspend fun openTunnel(
        relayUrl: String,
        serverId: String,
        relayToken: String,
        clientId: String,
        clientVersion: String
    ): RelayTunnel = withContext(Dispatchers.IO) {
        Timber.i("Opening relay tunnel to $serverId via $relayUrl")

        val connectUrl = "${relayUrl.trimEnd('/')}/connect"
        val tunnelReady = CompletableDeferred<RelayTunnel>()
        val incomingData = Channel<ByteArray>(capacity = Channel.BUFFERED)

        var healthMonitor: TunnelHealthMonitor? = null

        val listener = object : WebSocketListener() {
            private var tunnelId: String? = null
            private var dataMode = false

            override fun onOpen(webSocket: WebSocket, response: Response) {
                Timber.d("WebSocket opened, sending connect message")

                val connectMsg = ConnectMessage(
                    server_id = serverId,
                    relay_token = relayToken,
                    client_id = clientId,
                    client_version = clientVersion
                )

                val msgJson = json.encodeToString(ConnectMessage.serializer(), connectMsg)
                webSocket.send(msgJson)
            }

            override fun onMessage(webSocket: WebSocket, text: String) {
                healthMonitor?.recordActivity()
                Timber.v("Tunnel $tunnelId received text message, activity recorded")

                if (dataMode) {
                    Timber.w("Received unexpected text message in data mode: ${text.take(100)}")
                    return
                }

                try {
                    // Parse control messages
                    val ack = json.decodeFromString<ConnectAck>(text)

                    when (ack.type) {
                        "connect_ack" -> {
                            if (ack.status == "ok" && ack.tunnel_id != null) {
                                tunnelId = ack.tunnel_id
                                Timber.i("Relay acknowledged connection, tunnel_id=${ack.tunnel_id}")

                                // Create health monitor
                                val monitor = TunnelHealthMonitor(scope) {
                                    Timber.e("Tunnel ${ack.tunnel_id} became unhealthy, closing connection")
                                    incomingData.close(IOException("Tunnel health check failed"))
                                    webSocket.close(1001, "Health check timeout")
                                }
                                healthMonitor = monitor

                                // Send tunnel_ready
                                val readyMsg = TunnelReadyMessage(
                                    tunnel_id = ack.tunnel_id
                                )
                                val readyJson = json.encodeToString(
                                    TunnelReadyMessage.serializer(),
                                    readyMsg
                                )
                                webSocket.send(readyJson)

                                // Switch to binary data mode
                                dataMode = true

                                // Complete the tunnel setup
                                val tunnel = RelayTunnel(
                                    tunnelId = ack.tunnel_id,
                                    webSocket = webSocket,
                                    incomingData = incomingData,
                                    relayHost = ack.relay_host,
                                    serverAuthority = ack.server_authority,
                                    healthMonitor = monitor
                                )

                                // Start health monitoring
                                monitor.start()

                                tunnelReady.complete(tunnel)
                            } else {
                                val errorMsg = "Relay connection rejected: ${ack.status}"
                                Timber.e(errorMsg)
                                tunnelReady.completeExceptionally(
                                    RelayConnectionException(errorMsg)
                                )
                                webSocket.close(1000, "Connection rejected")
                            }
                        }
                        else -> {
                            Timber.w("Unknown relay message type: ${ack.type}")
                        }
                    }
                } catch (e: Exception) {
                    Timber.e(e, "Failed to parse relay control message")
                    tunnelReady.completeExceptionally(
                        RelayConnectionException("Protocol error: ${e.message}", e)
                    )
                    webSocket.close(1002, "Protocol error")
                }
            }

            override fun onMessage(webSocket: WebSocket, bytes: ByteString) {
                healthMonitor?.recordActivity()
                Timber.v("Tunnel $tunnelId received ${bytes.size} bytes, activity recorded")

                if (!dataMode) {
                    Timber.w("Received binary data before tunnel ready")
                    return
                }

                // Forward binary data from relay to gRPC
                scope.launch {
                    try {
                        incomingData.send(bytes.toByteArray())
                    } catch (e: Exception) {
                        Timber.w(e, "Failed to forward binary data, channel closed")
                    }
                }
            }

            override fun onFailure(webSocket: WebSocket, t: Throwable, response: Response?) {
                val statusCode = response?.code
                val errorMsg = when {
                    statusCode != null -> "WebSocket failed with HTTP $statusCode: ${response.message}"
                    t is IOException && t.message?.contains("Software caused connection abort") == true ->
                        "Connection lost: network error or server disconnected"
                    else -> "WebSocket connection failed: ${t.message ?: t.javaClass.simpleName}"
                }

                Timber.e(t, errorMsg)

                if (!tunnelReady.isCompleted) {
                    tunnelReady.completeExceptionally(
                        RelayConnectionException(errorMsg, t)
                    )
                } else {
                    // Tunnel was established but failed later
                    healthMonitor?.stop()
                }

                incomingData.close(IOException(errorMsg, t))
            }

            override fun onClosed(webSocket: WebSocket, code: Int, reason: String) {
                Timber.i("WebSocket closed: code=$code, reason=$reason")
                healthMonitor?.stop()
                incomingData.close()
            }
        }

        val request = Request.Builder()
            .url(connectUrl)
            .addHeader("Sec-WebSocket-Protocol", "handcontrol-relay.v1")
            .build()

        val webSocket = okHttpClient.newWebSocket(request, listener)

        try {
            tunnelReady.await()
        } catch (e: Exception) {
            webSocket.close(1000, "Failed to establish tunnel")
            throw e
        }
    }

    fun shutdown() {
        okHttpClient.dispatcher.executorService.shutdown()
        okHttpClient.connectionPool.evictAll()
    }
}

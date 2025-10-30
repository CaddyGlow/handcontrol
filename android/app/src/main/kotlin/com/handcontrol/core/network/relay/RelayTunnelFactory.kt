package com.handcontrol.core.network.relay

import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.channels.Channel
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
import java.util.concurrent.TimeUnit
import javax.inject.Inject
import javax.inject.Singleton

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
    val expires_at: Long? = null
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
    val authority: String
) {
    suspend fun sendData(data: ByteArray) {
        webSocket.send(ByteString.of(*data))
    }

    fun close() {
        webSocket.close(1000, "Tunnel closed")
        incomingData.close()
    }
}

/**
 * Factory for creating WebSocket-based relay tunnels
 */
@Singleton
class RelayTunnelFactory @Inject constructor() {

    private val json = Json { ignoreUnknownKeys = true }

    private val okHttpClient = OkHttpClient.Builder()
        .connectTimeout(10, TimeUnit.SECONDS)
        .readTimeout(0, TimeUnit.SECONDS) // No read timeout for persistent connection
        .writeTimeout(10, TimeUnit.SECONDS)
        .pingInterval(30, TimeUnit.SECONDS)
        .build()

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
                                tunnelReady.complete(
                                    RelayTunnel(
                                        tunnelId = ack.tunnel_id,
                                        webSocket = webSocket,
                                        incomingData = incomingData,
                                        authority = ack.relay_host ?: relayUrl
                                    )
                                )
                            } else {
                                tunnelReady.completeExceptionally(
                                    IOException("Relay connection failed: ${ack.status}")
                                )
                                webSocket.close(1000, "Connection rejected")
                            }
                        }
                        else -> {
                            Timber.w("Unknown message type: ${ack.type}")
                        }
                    }
                } catch (e: Exception) {
                    Timber.e(e, "Failed to parse relay control message")
                    tunnelReady.completeExceptionally(e)
                    webSocket.close(1002, "Protocol error")
                }
            }

            override fun onMessage(webSocket: WebSocket, bytes: ByteString) {
                if (!dataMode) {
                    Timber.w("Received binary data before tunnel ready")
                    return
                }

                // Forward binary data from relay to gRPC
                val success = incomingData.trySend(bytes.toByteArray())
                if (!success.isSuccess) {
                    Timber.w("Failed to forward binary data, channel full or closed")
                }
            }

            override fun onFailure(webSocket: WebSocket, t: Throwable, response: Response?) {
                Timber.e(t, "WebSocket failure: ${response?.message}")
                if (!tunnelReady.isCompleted) {
                    tunnelReady.completeExceptionally(t)
                }
                incomingData.close(t)
            }

            override fun onClosed(webSocket: WebSocket, code: Int, reason: String) {
                Timber.i("WebSocket closed: code=$code, reason=$reason")
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

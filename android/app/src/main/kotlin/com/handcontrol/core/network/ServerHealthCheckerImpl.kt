package com.handcontrol.core.network

import com.handcontrol.core.model.HealthCheckResult
import com.handcontrol.data.database.EnrolledServerEntity
import com.handcontrol.data.database.EnrolledServerRepository
import com.handcontrol.grpc.RemoteControlGrpcKt
import com.handcontrol.grpc.ServerInfoRequest
import io.grpc.StatusException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeout
import timber.log.Timber
import javax.inject.Inject
import javax.inject.Singleton

@Singleton
class ServerHealthCheckerImpl @Inject constructor(
    private val connectionManager: ServerConnectionManager,
    private val enrolledServerRepository: EnrolledServerRepository
) : ServerHealthChecker {

    override suspend fun checkHealth(server: EnrolledServerEntity): HealthCheckResult = withContext(Dispatchers.IO) {
        Timber.d("Starting health check for ${server.serverName} (${server.serverId})")
        val startTime = System.currentTimeMillis()

        try {
            // Use a timeout for the entire health check (including connection attempts)
            withTimeout(10000L) {
                // Use ServerConnectionManager to connect (tries direct, then relay)
                val connectionResult = connectionManager.connect(server)

                try {
                    val stub = RemoteControlGrpcKt.RemoteControlCoroutineStub(connectionResult.channel)

                    // Call GetServerInfo as a lightweight health check
                    stub.getServerInfo(ServerInfoRequest.getDefaultInstance())

                    val latency = System.currentTimeMillis() - startTime

                    Timber.i(
                        "Health check successful for ${server.serverName} via ${connectionResult.mode}, " +
                        "latency=${latency}ms, address=${connectionResult.connectedAddress ?: "relay"}"
                    )

                    // Persist successful connection mode
                    enrolledServerRepository.updateConnectionMode(server.serverId, connectionResult.mode)

                    HealthCheckResult(
                        isReachable = true,
                        latencyMs = latency
                    )
                } finally {
                    // Leave relay connections cached for reuse; allow graceful shutdown otherwise
                    connectionManager.disconnect(connectionResult)
                }
            }
        } catch (e: RelayConnectionException) {
            Timber.w("Health check relay error for ${server.serverName}: ${e.shortMessage}")
            HealthCheckResult(
                isReachable = false,
                error = e.userMessage
            )
        } catch (e: StatusException) {
            val errorMsg = when (e.status.code) {
                io.grpc.Status.Code.UNAVAILABLE -> "Server unavailable"
                io.grpc.Status.Code.DEADLINE_EXCEEDED -> "Connection timeout"
                io.grpc.Status.Code.UNAUTHENTICATED -> "Authentication failed"
                else -> e.status.description ?: "gRPC error: ${e.status.code}"
            }
            Timber.w("Health check gRPC error for ${server.serverName}: $errorMsg")
            HealthCheckResult(
                isReachable = false,
                error = errorMsg
            )
        } catch (e: Exception) {
            Timber.w("Health check failed for ${server.serverName}: ${e.message}")
            HealthCheckResult(
                isReachable = false,
                error = e.message ?: "Connection failed"
            )
        }
    }
}

package com.handcontrol.core.network

import com.handcontrol.core.model.HealthCheckResult
import com.handcontrol.data.database.EnrolledServerEntity
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
    private val channelFactory: MtlsGrpcChannelFactory
) : ServerHealthChecker {

    override suspend fun checkHealth(server: EnrolledServerEntity): HealthCheckResult = withContext(Dispatchers.IO) {
        // Try primary IP first, then fallback to other IPs
        val ipAddresses = if (server.ips.isNotEmpty()) {
            server.ips
        } else {
            // Fallback to serverHost for backward compatibility
            server.serverHost?.let { listOf(it) } ?: emptyList()
        }

        if (ipAddresses.isEmpty()) {
            Timber.w("No IP addresses available for server ${server.serverId}")
            return@withContext HealthCheckResult(
                isReachable = false,
                error = "No IP addresses configured"
            )
        }

        // Try each IP address until one succeeds
        for ((index, ip) in ipAddresses.withIndex()) {
            try {
                Timber.d("Health check attempt ${index + 1}/${ipAddresses.size} for ${server.serverName} at $ip:${server.serverPort}")
                return@withContext checkSingleHost(ip, server.serverPort)
            } catch (e: Exception) {
                Timber.d("Health check failed for $ip: ${e.message}")
                if (index == ipAddresses.size - 1) {
                    // Last IP failed, return error
                    return@withContext HealthCheckResult(
                        isReachable = false,
                        error = e.message ?: "Connection failed"
                    )
                }
                // Try next IP
            }
        }

        // Should not reach here, but just in case
        HealthCheckResult(
            isReachable = false,
            error = "All connection attempts failed"
        )
    }

    private suspend fun checkSingleHost(host: String, port: Int): HealthCheckResult {
        val startTime = System.currentTimeMillis()
        var channel: io.grpc.ManagedChannel? = null

        return try {
            // Use a short timeout for health checks (3 seconds)
            withTimeout(3000L) {
                channel = channelFactory.createChannel(host, port)
                val stub = RemoteControlGrpcKt.RemoteControlCoroutineStub(channel!!)

                // Call GetServerInfo as a lightweight health check
                stub.getServerInfo(ServerInfoRequest.getDefaultInstance())

                val latency = System.currentTimeMillis() - startTime
                Timber.d("Health check successful for $host:$port, latency=${latency}ms")

                HealthCheckResult(
                    isReachable = true,
                    latencyMs = latency
                )
            }
        } catch (e: StatusException) {
            val errorMsg = when (e.status.code) {
                io.grpc.Status.Code.UNAVAILABLE -> "Server unavailable"
                io.grpc.Status.Code.DEADLINE_EXCEEDED -> "Connection timeout"
                io.grpc.Status.Code.UNAUTHENTICATED -> "Authentication failed"
                else -> e.status.description ?: "gRPC error: ${e.status.code}"
            }
            Timber.w("Health check gRPC error for $host:$port: $errorMsg")
            HealthCheckResult(
                isReachable = false,
                error = errorMsg
            )
        } catch (e: Exception) {
            Timber.w("Health check failed for $host:$port: ${e.message}")
            HealthCheckResult(
                isReachable = false,
                error = e.message ?: "Connection failed"
            )
        } finally {
            channel?.let {
                try {
                    channelFactory.shutdownChannel(it)
                } catch (e: Exception) {
                    Timber.w(e, "Error shutting down health check channel")
                }
            }
        }
    }
}

package com.handcontrol.core.model

import com.handcontrol.data.database.ConnectionMode

/**
 * Represents the result of a health check on a server.
 */
data class HealthCheckResult(
    val isReachable: Boolean,
    val latencyMs: Long? = null,
    val error: String? = null,
    val checkedAt: Long = System.currentTimeMillis()
)

/**
 * Represents the overall health status of a server.
 */
sealed class ServerHealthStatus {
    /**
     * Health check is in progress.
     */
    data object Checking : ServerHealthStatus()

    /**
     * Server is connected and reachable.
     * @param latencyMs Round-trip latency in milliseconds.
     */
    data class Connected(val latencyMs: Long) : ServerHealthStatus()

    /**
     * Server is not reachable or connection failed.
     * @param reason Optional error message describing the failure.
     */
    data class Disconnected(val reason: String? = null) : ServerHealthStatus()

    /**
     * Health check has not been performed yet.
     */
    data object Unknown : ServerHealthStatus()
}

/**
 * Comprehensive server detail information combining database entity,
 * health status, and mDNS discovery state.
 */
data class ServerDetailInfo(
    val serverId: String,
    val serverName: String,
    val ips: List<String>,
    val serverPort: Int,
    val clientId: String,
    val certFingerprint: String,
    val enrolledAt: Long,
    val lastConnected: Long?,
    val lastConnectionMode: ConnectionMode,
    val relayEnabled: Boolean = false,
    val relayUrl: String? = null,
    val healthStatus: ServerHealthStatus = ServerHealthStatus.Unknown,
    val isDiscoveredViaMdns: Boolean = false,
    val mdnsServiceName: String? = null,
    val mdnsLastSeen: Long? = null
)

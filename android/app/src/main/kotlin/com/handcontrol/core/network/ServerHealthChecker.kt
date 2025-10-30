package com.handcontrol.core.network

import com.handcontrol.core.model.HealthCheckResult
import com.handcontrol.data.database.EnrolledServerEntity

/**
 * Interface for checking server health and connectivity.
 */
interface ServerHealthChecker {
    /**
     * Performs a health check on the given server.
     * Measures round-trip latency and determines if the server is reachable.
     *
     * @param server The enrolled server to check.
     * @return HealthCheckResult containing reachability status, latency, and any errors.
     */
    suspend fun checkHealth(server: EnrolledServerEntity): HealthCheckResult
}

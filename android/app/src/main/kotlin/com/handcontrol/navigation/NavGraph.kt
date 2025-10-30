package com.handcontrol.navigation

import kotlinx.serialization.Serializable

/**
 * Navigation routes for the app using type-safe navigation
 */
sealed interface Route {
    @Serializable
    data object Welcome : Route

    @Serializable
    data object ServerDiscovery : Route

    @Serializable
    data object ServerList : Route

    @Serializable
    data class ServerDetails(val serverId: String) : Route

    @Serializable
    data class EnrollmentQr(val serverHost: String, val serverPort: Int) : Route

    @Serializable
    data class EnrollmentApproval(
        val serverHost: String,
        val serverPort: Int,
        val serverId: String? = null
    ) : Route

    @Serializable
    data class CommandList(val serverId: String) : Route

    @Serializable
    data class CommandDetail(
        val serverId: String,
        val commandId: String
    ) : Route

    @Serializable
    data class CommandExecution(
        val serverId: String,
        val commandId: String
    ) : Route
}

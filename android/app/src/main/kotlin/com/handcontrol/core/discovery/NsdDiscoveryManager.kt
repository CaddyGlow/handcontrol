package com.handcontrol.core.discovery

import kotlinx.coroutines.flow.Flow

data class DiscoveredServer(
    val name: String,
    val host: String,
    val port: Int,
    val fingerprint: String?,
    val serverId: String?,
    val version: String?
)

interface NsdDiscoveryManager {
    val servers: Flow<List<DiscoveredServer>>
    suspend fun startDiscovery()
    suspend fun stopDiscovery()
}

package com.handcontrol.data.database

import com.handcontrol.feature.commands.remote.RemoteLayoutSpec
import kotlinx.coroutines.flow.Flow
import javax.inject.Inject
import javax.inject.Singleton

@Singleton
class EnrolledServerRepository @Inject constructor(
    private val enrolledServerDao: EnrolledServerDao
) {
    val allServers: Flow<List<EnrolledServerEntity>> =
        enrolledServerDao.getAllServers()

    val lastConnectedServer: Flow<EnrolledServerEntity?> =
        enrolledServerDao.observeLastConnectedServer()

    suspend fun saveServer(
        serverId: String,
        serverHost: String,
        serverPort: Int,
        clientId: String,
        serverName: String,
        certFingerprint: String,
        relayEnabled: Boolean = false,
        relayUrl: String? = null,
        relayToken: String? = null,
        initialConnectionMode: ConnectionMode = ConnectionMode.UNKNOWN
    ) {
        // Convert single host to list for backward compatibility
        val server = EnrolledServerEntity(
            serverId = serverId,
            ips = listOf(serverHost),
            serverPort = serverPort,
            clientId = clientId,
            serverName = serverName,
            certFingerprint = certFingerprint,
            enrolledAt = System.currentTimeMillis(),
            lastConnected = System.currentTimeMillis(),
            serverHost = serverHost,  // Keep for migration compatibility
            relayEnabled = relayEnabled,
            relayUrl = relayUrl,
            relayToken = relayToken,
            connectionPreference = ConnectionPreference.AUTO,
            lastConnectionMode = initialConnectionMode
        )
        enrolledServerDao.insertServer(server)
    }

    suspend fun updateLastConnected(serverId: String) {
        enrolledServerDao.updateLastConnected(serverId, System.currentTimeMillis())
    }

    suspend fun updateConnectionMode(serverId: String, mode: ConnectionMode) {
        enrolledServerDao.updateConnectionMode(serverId, System.currentTimeMillis(), mode)
    }

    suspend fun updateConnectionPreference(serverId: String, preference: ConnectionPreference) {
        enrolledServerDao.updateConnectionPreference(serverId, preference)
    }

    suspend fun updateServerFingerprint(serverId: String, fingerprint: String) {
        enrolledServerDao.updateCertFingerprint(serverId, fingerprint)
    }

    suspend fun removeServer(serverId: String) {
        enrolledServerDao.deleteServerById(serverId)
    }

    suspend fun deleteAllServers() {
        enrolledServerDao.deleteAllServers()
    }

    suspend fun getServerById(serverId: String): EnrolledServerEntity? {
        return enrolledServerDao.getServerById(serverId)
    }

    fun observeServerById(serverId: String): Flow<EnrolledServerEntity?> {
        return enrolledServerDao.observeServerById(serverId)
    }

    suspend fun getServerFingerprint(serverId: String): String? {
        return enrolledServerDao.getServerFingerprint(serverId)
    }

    suspend fun getLastConnectedServer(): EnrolledServerEntity? {
        return enrolledServerDao.getLastConnectedServer()
    }

    suspend fun isServerAlreadyEnrolled(host: String, port: Int): Boolean {
        return enrolledServerDao.getServerByHostAndPort(host, port) != null
    }

    suspend fun getServerByHostAndPort(host: String, port: Int): EnrolledServerEntity? {
        return enrolledServerDao.getServerByHostAndPort(host, port)
    }

    suspend fun mergeServerIps(serverId: String, discoveredIps: List<String>) {
        enrolledServerDao.mergeServerIpsTransactional(serverId, discoveredIps)
    }

    fun observeRemoteLayoutSpec(serverId: String): Flow<RemoteLayoutSpec?> {
        return enrolledServerDao.observeRemoteLayoutSpec(serverId)
    }

    suspend fun getRemoteLayoutSpec(serverId: String): RemoteLayoutSpec? {
        return enrolledServerDao.getRemoteLayoutSpec(serverId)
    }

    suspend fun saveRemoteLayoutSpec(serverId: String, spec: RemoteLayoutSpec?) {
        enrolledServerDao.updateRemoteLayoutSpec(serverId, spec)
    }
}

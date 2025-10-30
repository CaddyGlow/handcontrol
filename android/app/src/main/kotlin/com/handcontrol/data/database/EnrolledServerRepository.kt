package com.handcontrol.data.database

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
        certFingerprint: String
    ) {
        val server = EnrolledServerEntity(
            serverId = serverId,
            serverHost = serverHost,
            serverPort = serverPort,
            clientId = clientId,
            serverName = serverName,
            certFingerprint = certFingerprint,
            enrolledAt = System.currentTimeMillis(),
            lastConnected = System.currentTimeMillis()
        )
        enrolledServerDao.insertServer(server)
    }

    suspend fun updateLastConnected(serverId: String) {
        enrolledServerDao.updateLastConnected(serverId, System.currentTimeMillis())
    }

    suspend fun removeServer(serverId: String) {
        enrolledServerDao.deleteServerById(serverId)
    }

    suspend fun getServerById(serverId: String): EnrolledServerEntity? {
        return enrolledServerDao.getServerById(serverId)
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
}

package com.handcontrol.data.database

import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class EnrolledServerDaoMergeIpsTest {

    @Test
    fun `mergeServerIpsTransactional adds new sanitized addresses`() = runTest {
        val existing = createServer(
            serverId = "server-1",
            ips = listOf("192.168.1.10"),
            serverHost = "192.168.1.10"
        )
        val dao = FakeEnrolledServerDao(existing)

        dao.mergeServerIpsTransactional(
            serverId = "server-1",
            discoveredIps = listOf(" 10.0.0.2 ", "unknown", "", "192.168.1.10")
        )

        val merged = dao.servers["server-1"]!!
        assertEquals(listOf("192.168.1.10", "10.0.0.2"), merged.ips)
        assertEquals("192.168.1.10", merged.serverHost)
        assertEquals(1, dao.updateCalls.size)
    }

    @Test
    fun `mergeServerIpsTransactional skips update when addresses unchanged`() = runTest {
        val existing = createServer(
            serverId = "server-1",
            ips = listOf("192.168.1.10"),
            serverHost = "192.168.1.10"
        )
        val dao = FakeEnrolledServerDao(existing)

        dao.mergeServerIpsTransactional(
            serverId = "server-1",
            discoveredIps = listOf("192.168.1.10", "192.168.1.10")
        )

        assertEquals(existing, dao.servers["server-1"])
        assertTrue(dao.updateCalls.isEmpty())
    }

    @Test
    fun `mergeServerIpsTransactional assigns primary host when none is stored`() = runTest {
        val existing = createServer(
            serverId = "server-2",
            ips = emptyList(),
            serverHost = null
        )
        val dao = FakeEnrolledServerDao(existing)

        dao.mergeServerIpsTransactional(
            serverId = "server-2",
            discoveredIps = listOf("fd00::1")
        )

        val merged = dao.servers["server-2"]!!
        assertEquals(listOf("fd00::1"), merged.ips)
        assertEquals("fd00::1", merged.serverHost)
        assertEquals(1, dao.updateCalls.size)
    }

    private fun createServer(
        serverId: String,
        ips: List<String>,
        serverHost: String?
    ): EnrolledServerEntity {
        return EnrolledServerEntity(
            serverId = serverId,
            ips = ips,
            serverPort = 50051,
            clientId = "client-$serverId",
            serverName = "Server $serverId",
            certFingerprint = "fingerprint-$serverId",
            enrolledAt = 1_700_000_000_000,
            lastConnected = null,
            relayEnabled = false,
            relayUrl = null,
            relayToken = null,
            lastConnectionMode = ConnectionMode.UNKNOWN,
            serverHost = serverHost
        )
    }

    private class FakeEnrolledServerDao(
        initial: EnrolledServerEntity
    ) : EnrolledServerDao {
        val servers = mutableMapOf(initial.serverId to initial)
        val updateCalls = mutableListOf<Triple<String, List<String>, String?>>()

        override fun getAllServers(): Flow<List<EnrolledServerEntity>> {
            throw NotImplementedError("Not needed for this test")
        }

        override suspend fun getServerById(serverId: String): EnrolledServerEntity? {
            return servers[serverId]
        }

        override fun observeServerById(serverId: String): Flow<EnrolledServerEntity?> {
            throw NotImplementedError("Not needed for this test")
        }

        override fun observeRemoteLayoutSpec(serverId: String): Flow<com.handcontrol.feature.commands.remote.RemoteLayoutSpec?> {
            throw NotImplementedError("Not needed for this test")
        }

        override suspend fun getLastConnectedServer(): EnrolledServerEntity? {
            throw NotImplementedError("Not needed for this test")
        }

        override fun observeLastConnectedServer(): Flow<EnrolledServerEntity?> {
            throw NotImplementedError("Not needed for this test")
        }

        override suspend fun insertServer(server: EnrolledServerEntity) {
            throw NotImplementedError("Not needed for this test")
        }

        override suspend fun updateLastConnected(serverId: String, timestamp: Long) {
            throw NotImplementedError("Not needed for this test")
        }

        override suspend fun updateConnectionMode(serverId: String, timestamp: Long, mode: ConnectionMode) {
            throw NotImplementedError("Not needed for this test")
        }

        override suspend fun updateConnectionPreference(serverId: String, preference: ConnectionPreference) {
            throw NotImplementedError("Not needed for this test")
        }

        override suspend fun deleteServer(server: EnrolledServerEntity) {
            throw NotImplementedError("Not needed for this test")
        }

        override suspend fun deleteServerById(serverId: String) {
            throw NotImplementedError("Not needed for this test")
        }

        override suspend fun deleteAllServers() {
            throw NotImplementedError("Not needed for this test")
        }

        override suspend fun getServerFingerprint(serverId: String): String? {
            throw NotImplementedError("Not needed for this test")
        }

        override suspend fun updateCertFingerprint(serverId: String, fingerprint: String) {
            throw NotImplementedError("Not needed for this test")
        }

        override suspend fun getServerByHostAndPort(host: String, port: Int): EnrolledServerEntity? {
            throw NotImplementedError("Not needed for this test")
        }

        override suspend fun getRemoteLayoutSpec(serverId: String): com.handcontrol.feature.commands.remote.RemoteLayoutSpec? {
            throw NotImplementedError("Not needed for this test")
        }

        override suspend fun updateRemoteLayoutSpec(serverId: String, layoutSpec: com.handcontrol.feature.commands.remote.RemoteLayoutSpec?) {
            throw NotImplementedError("Not needed for this test")
        }

        override suspend fun updateServerIps(serverId: String, ips: List<String>, primaryHost: String?) {
            val existing = servers[serverId] ?: return
            val updated = existing.copy(ips = ips, serverHost = primaryHost)
            servers[serverId] = updated
            updateCalls += Triple(serverId, ips, primaryHost)
        }
    }
}

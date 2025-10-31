package com.handcontrol.data.database

import io.mockk.Runs
import io.mockk.coEvery
import io.mockk.coVerify
import io.mockk.just
import io.mockk.mockk
import io.mockk.slot
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test

class EnrolledServerRepositoryTest {

    private lateinit var enrolledServerDao: EnrolledServerDao
    private lateinit var repository: EnrolledServerRepository

    @Before
    fun setup() {
        enrolledServerDao = mockk()
        repository = EnrolledServerRepository(enrolledServerDao)
    }

    @Test
    fun `updateConnectionMode calls DAO with correct parameters for DIRECT mode`() = runTest {
        // Given
        val serverId = "test-server-id"
        val mode = ConnectionMode.DIRECT
        val timestampSlot = slot<Long>()

        coEvery {
            enrolledServerDao.updateConnectionMode(serverId, capture(timestampSlot), mode)
        } just Runs

        val timeBefore = System.currentTimeMillis()

        // When
        repository.updateConnectionMode(serverId, mode)

        val timeAfter = System.currentTimeMillis()

        // Then
        coVerify(exactly = 1) {
            enrolledServerDao.updateConnectionMode(serverId, any(), mode)
        }

        // Verify timestamp is recent (within test execution window)
        assertTrue(
            "Timestamp should be between test start and end",
            timestampSlot.captured in timeBefore..timeAfter
        )
    }

    @Test
    fun `updateConnectionMode calls DAO with correct parameters for RELAY mode`() = runTest {
        // Given
        val serverId = "test-server-id"
        val mode = ConnectionMode.RELAY

        coEvery {
            enrolledServerDao.updateConnectionMode(serverId, any(), mode)
        } just Runs

        // When
        repository.updateConnectionMode(serverId, mode)

        // Then
        coVerify(exactly = 1) {
            enrolledServerDao.updateConnectionMode(serverId, any(), mode)
        }
    }

    @Test
    fun `updateConnectionMode calls DAO with correct parameters for UNKNOWN mode`() = runTest {
        // Given
        val serverId = "test-server-id"
        val mode = ConnectionMode.UNKNOWN

        coEvery {
            enrolledServerDao.updateConnectionMode(serverId, any(), mode)
        } just Runs

        // When
        repository.updateConnectionMode(serverId, mode)

        // Then
        coVerify(exactly = 1) {
            enrolledServerDao.updateConnectionMode(serverId, any(), mode)
        }
    }

    @Test
    fun `updateConnectionMode updates timestamp on each call`() = runTest {
        // Given
        val serverId = "test-server-id"
        val mode = ConnectionMode.DIRECT
        val timestamps = mutableListOf<Long>()

        coEvery {
            enrolledServerDao.updateConnectionMode(serverId, capture(timestamps), mode)
        } just Runs

        // When - call twice with a delay
        repository.updateConnectionMode(serverId, mode)
        Thread.sleep(10) // Small delay to ensure different timestamps
        repository.updateConnectionMode(serverId, mode)

        // Then
        assertEquals(2, timestamps.size)
        assertTrue(
            "Second timestamp should be after first",
            timestamps[1] > timestamps[0]
        )
    }

    @Test
    fun `getServerById delegates to DAO`() = runTest {
        // Given
        val serverId = "test-server-id"
        val expectedServer = EnrolledServerEntity(
            serverId = serverId,
            ips = listOf("192.168.1.100"),
            serverPort = 50051,
            clientId = "client-id",
            serverName = "Test Server",
            certFingerprint = "SHA256:test-fingerprint",
            enrolledAt = System.currentTimeMillis(),
            lastConnected = null,
            relayEnabled = true,
            relayUrl = "wss://relay.example.com",
            relayToken = "test-token",
            lastConnectionMode = ConnectionMode.DIRECT
        )

        coEvery {
            enrolledServerDao.getServerById(serverId)
        } returns expectedServer

        // When
        val result = repository.getServerById(serverId)

        // Then
        assertEquals(expectedServer, result)
        coVerify(exactly = 1) {
            enrolledServerDao.getServerById(serverId)
        }
    }

    @Test
    fun `getServerById returns null when server not found`() = runTest {
        // Given
        val serverId = "non-existent-id"

        coEvery {
            enrolledServerDao.getServerById(serverId)
        } returns null

        // When
        val result = repository.getServerById(serverId)

        // Then
        assertEquals(null, result)
    }
}

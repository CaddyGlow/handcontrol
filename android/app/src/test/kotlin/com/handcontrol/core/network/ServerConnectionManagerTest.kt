package com.handcontrol.core.network

import com.handcontrol.core.network.relay.RelayGrpcChannelFactory
import com.handcontrol.data.database.ConnectionMode
import com.handcontrol.data.database.EnrolledServerEntity
import io.grpc.ConnectivityState
import io.grpc.ManagedChannel
import io.mockk.coEvery
import io.mockk.coVerify
import io.mockk.every
import io.mockk.mockk
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.fail
import org.junit.Before
import org.junit.Test

class ServerConnectionManagerTest {

    private lateinit var directChannelFactory: MtlsGrpcChannelFactory
    private lateinit var relayChannelFactory: RelayGrpcChannelFactory
    private lateinit var connectionManager: ServerConnectionManager

    private lateinit var mockDirectChannel: ManagedChannel
    private lateinit var mockRelayChannel: ManagedChannel
    private lateinit var testServer: EnrolledServerEntity

    @Before
    fun setup() {
        directChannelFactory = mockk()
        relayChannelFactory = mockk()
        connectionManager = ServerConnectionManager(directChannelFactory, relayChannelFactory)

        mockDirectChannel = mockk(relaxed = true)
        mockRelayChannel = mockk(relaxed = true)

        every { mockDirectChannel.getState(any()) } returns ConnectivityState.READY
        coEvery { directChannelFactory.forceShutdownChannel(any()) } returns Unit

        testServer = EnrolledServerEntity(
            serverId = "test-server-id",
            ips = listOf("192.168.1.100", "192.168.1.101"),
            serverPort = 50051,
            clientId = "test-client-id",
            serverName = "Test Server",
            certFingerprint = "SHA256:test-fingerprint",
            enrolledAt = System.currentTimeMillis(),
            lastConnected = null,
            relayEnabled = true,
            relayUrl = "wss://relay.example.com",
            relayToken = "test-relay-token",
            lastConnectionMode = ConnectionMode.UNKNOWN
        )
    }

    @Test
    fun `connect succeeds via direct connection on first IP`() = runTest {
        // Given
        coEvery {
            directChannelFactory.createChannel("192.168.1.100", 50051)
        } returns mockDirectChannel

        // When
        val result = connectionManager.connect(testServer)

        // Then
        assertEquals(ConnectionMode.DIRECT, result.mode)
        assertEquals(mockDirectChannel, result.channel)
        assertEquals("192.168.1.100:50051", result.connectedAddress)

        coVerify(exactly = 1) {
            directChannelFactory.createChannel("192.168.1.100", 50051)
        }
        coVerify(exactly = 0) {
            relayChannelFactory.createChannelViaRelay(any(), any(), any(), any())
        }
    }

    @Test
    fun `connect succeeds via direct connection on second IP after first fails`() = runTest {
        // Given
        coEvery {
            directChannelFactory.createChannel("192.168.1.100", 50051)
        } throws Exception("Connection timeout")

        coEvery {
            directChannelFactory.createChannel("192.168.1.101", 50051)
        } returns mockDirectChannel

        // When
        val result = connectionManager.connect(testServer)

        // Then
        assertEquals(ConnectionMode.DIRECT, result.mode)
        assertEquals(mockDirectChannel, result.channel)
        assertEquals("192.168.1.101:50051", result.connectedAddress)

        coVerify(exactly = 1) {
            directChannelFactory.createChannel("192.168.1.100", 50051)
        }
        coVerify(exactly = 1) {
            directChannelFactory.createChannel("192.168.1.101", 50051)
        }
        coVerify(exactly = 0) {
            relayChannelFactory.createChannelViaRelay(any(), any(), any(), any())
        }
    }

    @Test
    fun `connect falls back to relay when all direct connections fail`() = runTest {
        // Given
        coEvery {
            directChannelFactory.createChannel(any(), any())
        } throws Exception("Connection timeout")

        coEvery {
            relayChannelFactory.createChannelViaRelay(
                relayUrl = "wss://relay.example.com",
                serverId = "test-server-id",
                relayToken = "test-relay-token",
                clientId = "test-client-id"
            )
        } returns mockRelayChannel

        // When
        val result = connectionManager.connect(testServer)

        // Then
        assertEquals(ConnectionMode.RELAY, result.mode)
        assertEquals(mockRelayChannel, result.channel)
        assertEquals(null, result.connectedAddress)

        coVerify(exactly = 1) {
            directChannelFactory.createChannel("192.168.1.100", 50051)
        }
        coVerify(exactly = 1) {
            directChannelFactory.createChannel("192.168.1.101", 50051)
        }
        coVerify(exactly = 1) {
            relayChannelFactory.createChannelViaRelay(
                relayUrl = "wss://relay.example.com",
                serverId = "test-server-id",
                relayToken = "test-relay-token",
                clientId = "test-client-id"
            )
        }
    }

    @Test
    fun `connect throws exception when both direct and relay fail`() = runTest {
        // Given
        coEvery {
            directChannelFactory.createChannel(any(), any())
        } throws Exception("Connection timeout")

        coEvery {
            relayChannelFactory.createChannelViaRelay(any(), any(), any(), any())
        } throws Exception("Relay connection failed")

        // When/Then
        var captured: RelayConnectionException? = null
        try {
            connectionManager.connect(testServer)
            fail("Expected RelayConnectionException to be thrown")
        } catch (e: RelayConnectionException) {
            captured = e
        }

        val exception = captured ?: run {
            fail("Expected RelayConnectionException to be captured")
            return@runTest
        }
        assertEquals("Relay connection failed", exception.message)
    }

    @Test
    fun `connect throws exception when relay not configured and direct fails`() = runTest {
        // Given
        val serverWithoutRelay = testServer.copy(
            relayEnabled = false,
            relayUrl = null,
            relayToken = null
        )

        coEvery {
            directChannelFactory.createChannel(any(), any())
        } throws Exception("Connection timeout")

        // When/Then
        var captured: Exception? = null
        try {
            connectionManager.connect(serverWithoutRelay)
            fail("Expected exception to be thrown")
        } catch (e: Exception) {
            captured = e
        }

        val exception = captured ?: run {
            fail("Expected exception to be captured")
            return@runTest
        }
        assertEquals("All connection attempts failed", exception.message)

        coVerify(exactly = 0) {
            relayChannelFactory.createChannelViaRelay(any(), any(), any(), any())
        }
    }

    @Test
    fun `connect with preferRelay skips direct connection`() = runTest {
        // Given
        coEvery {
            relayChannelFactory.createChannelViaRelay(
                relayUrl = "wss://relay.example.com",
                serverId = "test-server-id",
                relayToken = "test-relay-token",
                clientId = "test-client-id"
            )
        } returns mockRelayChannel

        // When
        val result = connectionManager.connect(testServer, preferRelay = true)

        // Then
        assertEquals(ConnectionMode.RELAY, result.mode)
        assertEquals(mockRelayChannel, result.channel)

        coVerify(exactly = 0) {
            directChannelFactory.createChannel(any(), any())
        }
        coVerify(exactly = 1) {
            relayChannelFactory.createChannelViaRelay(any(), any(), any(), any())
        }
    }

    @Test
    fun `connect with preferRelay throws when relay not configured`() = runTest {
        // Given
        val serverWithoutRelay = testServer.copy(
            relayEnabled = false,
            relayUrl = null,
            relayToken = null
        )

        // When/Then
        var captured: RelayConnectionException? = null
        try {
            connectionManager.connect(serverWithoutRelay, preferRelay = true)
            fail("Expected RelayConnectionException to be thrown")
        } catch (e: RelayConnectionException) {
            captured = e
        }

        val exception = captured ?: run {
            fail("Expected RelayConnectionException to be captured")
            return@runTest
        }
        assertEquals("Relay connection not available", exception.message)

        coVerify(exactly = 0) {
            directChannelFactory.createChannel(any(), any())
        }
        coVerify(exactly = 0) {
            relayChannelFactory.createChannelViaRelay(any(), any(), any(), any())
        }
    }

    @Test
    fun `connect uses serverHost as fallback when ips list is empty`() = runTest {
        // Given
        val serverWithoutIps = testServer.copy(
            ips = emptyList(),
            serverHost = "example.com"
        )

        coEvery {
            directChannelFactory.createChannel("example.com", 50051)
        } returns mockDirectChannel

        // When
        val result = connectionManager.connect(serverWithoutIps)

        // Then
        assertEquals(ConnectionMode.DIRECT, result.mode)
        assertEquals("example.com:50051", result.connectedAddress)

        coVerify(exactly = 1) {
            directChannelFactory.createChannel("example.com", 50051)
        }
    }

    @Test
    fun `disconnect calls directChannelFactory for DIRECT mode`() = runTest {
        // Given
        val result = ConnectionResult(
            channel = mockDirectChannel,
            mode = ConnectionMode.DIRECT,
            connectedAddress = "192.168.1.100:50051"
        )

        coEvery {
            directChannelFactory.shutdownChannel(mockDirectChannel)
        } returns Unit

        // When
        connectionManager.disconnect(result)

        // Then
        coVerify(exactly = 1) {
            directChannelFactory.shutdownChannel(mockDirectChannel)
        }
        coVerify(exactly = 0) {
            relayChannelFactory.shutdownChannel(any())
        }
    }

    @Test
    fun `disconnect calls relayChannelFactory for RELAY mode`() = runTest {
        // Given
        val result = ConnectionResult(
            channel = mockRelayChannel,
            mode = ConnectionMode.RELAY,
            connectedAddress = null
        )

        coEvery {
            relayChannelFactory.shutdownChannel(mockRelayChannel)
        } returns Unit

        // When
        connectionManager.disconnect(result)

        // Then
        coVerify(exactly = 1) {
            relayChannelFactory.shutdownChannel(mockRelayChannel)
        }
        coVerify(exactly = 0) {
            directChannelFactory.shutdownChannel(any())
        }
    }

    @Test
    fun `disconnect handles UNKNOWN mode gracefully`() = runTest {
        // Given
        val result = ConnectionResult(
            channel = mockDirectChannel,
            mode = ConnectionMode.UNKNOWN,
            connectedAddress = null
        )

        coEvery {
            directChannelFactory.shutdownChannel(mockDirectChannel)
        } returns Unit

        // When
        connectionManager.disconnect(result)

        // Then
        coVerify(exactly = 1) {
            directChannelFactory.shutdownChannel(mockDirectChannel)
        }
    }

    @Test
    fun `shutdown calls both factories`() = runTest {
        // Given
        coEvery { directChannelFactory.shutdown() } returns Unit
        coEvery { relayChannelFactory.shutdown() } returns Unit

        // When
        connectionManager.shutdown()

        // Then
        coVerify(exactly = 1) { directChannelFactory.shutdown() }
        coVerify(exactly = 1) { relayChannelFactory.shutdown() }
    }
}

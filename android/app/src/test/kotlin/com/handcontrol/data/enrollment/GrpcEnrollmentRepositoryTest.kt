@file:OptIn(kotlinx.coroutines.ExperimentalCoroutinesApi::class)

package com.handcontrol.data.enrollment

import android.content.Context
import com.google.protobuf.ByteString
import com.handcontrol.core.network.MtlsGrpcChannelFactory
import com.handcontrol.core.network.RelayConnectionException
import com.handcontrol.core.network.relay.RelayGrpcChannelFactory
import com.handcontrol.core.security.ClientCertificate
import com.handcontrol.core.security.ClientCertificateManager
import com.handcontrol.core.security.VerificationCodeGenerator
import com.handcontrol.data.database.EnrolledServerRepository
import com.handcontrol.grpc.EnrollResponse
import com.handcontrol.grpc.RelayInfo
import io.grpc.ManagedChannel
import io.mockk.coEvery
import io.mockk.coVerify
import io.mockk.every
import io.mockk.mockk
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.IOException
import java.security.cert.X509Certificate
import java.time.Instant
import java.util.UUID

class GrpcEnrollmentRepositoryTest {

    @Test
    fun `relay fallback uses QR metadata when direct enrollment fails`() = runTest {
        val context = mockk<Context>(relaxed = true)
        val certificateManager = mockk<ClientCertificateManager>()
        val mtlsFactory = mockk<MtlsGrpcChannelFactory>(relaxed = true)
        val relayFactory = mockk<RelayGrpcChannelFactory>()
        val serverRepo = mockk<EnrolledServerRepository>(relaxed = true)

        val clientCertificate = ClientCertificate(
            certificateDer = byteArrayOf(1, 2, 3, 4),
            privateKeyAlias = "alias"
        )
        coEvery { certificateManager.loadOrCreate() } returns clientCertificate
        coEvery { certificateManager.pinServerFingerprint(any()) } returns Unit

        val relayChannel = mockk<ManagedChannel>(relaxed = true)
        val relayUrl = "wss://relay.example.com"
        val relayToken = "relay-token"
        val expectedServerId = "2c9c2c82-9f0c-4f2f-92b6-2c3c7a6d7f44"

        val handshakeCert = mockk<X509Certificate>()
        val handshakeDer = byteArrayOf(9, 9, 9, 9)
        every { handshakeCert.encoded } returns handshakeDer

        coEvery {
            relayFactory.createChannelViaRelay(
                relayUrl = relayUrl,
                serverId = expectedServerId,
                relayToken = relayToken,
                clientId = any(),
                defaultAuthority = any(),
                expectedFingerprint = any(),
                allowSelfSignedTls = false,
                pinnedCertSha256 = null
            )
        } returns relayChannel
        coEvery { relayFactory.getServerCertificate(relayChannel) } returns handshakeCert
        coEvery { relayFactory.shutdownChannel(relayChannel) } returns Unit

        val fingerprint = VerificationCodeGenerator.computeFingerprint(handshakeDer)
        val response = EnrollResponse.newBuilder()
            .setSuccess(true)
            .setClientId("client-123")
            .setRelayInfo(
                RelayInfo.newBuilder()
                    .setRelayUrl(relayUrl)
                    .setRelayToken(relayToken)
                    .setRelayRequired(true)
                    .build()
            )
            .build()

        val repository = object : GrpcEnrollmentRepository(
            context,
            certificateManager,
            mtlsFactory,
            relayFactory,
            serverRepo
        ) {
            var capturedRelayOptions: RelayEnrollmentOptions? = null

            override suspend fun openEnrollmentChannel(
                host: String,
                port: Int,
                expectedFingerprint: String?
            ): EnrollmentChannel {
                throw IOException("direct enrollment blocked for test")
            }

            override suspend fun sendEnrollmentRequest(
                channel: ManagedChannel,
                request: com.handcontrol.grpc.EnrollRequest
            ): EnrollResponse {
                // Ensure the request carries the expected certificate payload
                require(request.clientCertificate == ByteString.copyFrom(clientCertificate.certificateDer))
                return response
            }

            override suspend fun handleSuccessfulEnrollment(
                channel: ManagedChannel,
                response: EnrollResponse,
                fingerprint: String,
                expectedServerId: String,
                allHosts: List<String>,
                port: Int,
                relayOptions: RelayEnrollmentOptions?
            ): EnrollmentResult {
                capturedRelayOptions = relayOptions
                return EnrollmentResult.Success(response.clientId, expectedServerId)
            }
        }

        val qrRelayOptions = RelayEnrollmentOptions(
            relayUrl = relayUrl,
            relayToken = relayToken,
            relayRequired = true,
            allowSelfSignedTls = false,
            pinnedCertSha256 = null
        )

        val tokenUuid = UUID.randomUUID().toString()
        val result = repository.enrollWithToken(
            hosts = listOf("2001:db8::10"),
            port = 50051,
            token = tokenUuid,
            deviceName = "Test Device",
            expectedCertFingerprint = fingerprint,
            expectedServerId = expectedServerId,
            validUntil = Instant.now().plusSeconds(300),
            relayOptions = qrRelayOptions
        )

        assertTrue(result is EnrollmentResult.Success)
        val captured = repository.capturedRelayOptions
        assertNotNull("Relay options should be forwarded to success handler", captured)
        captured!!
        assertEquals(relayUrl, captured.relayUrl)
        assertEquals(relayToken, captured.relayToken)
        assertTrue(captured.relayRequired)

        coVerify {
            relayFactory.createChannelViaRelay(
                relayUrl = relayUrl,
                serverId = expectedServerId,
                relayToken = relayToken,
                clientId = tokenUuid,
                defaultAuthority = "[2001:db8::10]:50051",
                expectedFingerprint = fingerprint,
                allowSelfSignedTls = false,
                pinnedCertSha256 = null
            )
        }
    }

    @Test
    fun `direct enrollment succeeds without invoking relay`() = runTest {
        val context = mockk<Context>(relaxed = true)
        val certificateManager = mockk<ClientCertificateManager>()
        val mtlsFactory = mockk<MtlsGrpcChannelFactory>(relaxed = true)
        val relayFactory = mockk<RelayGrpcChannelFactory>(relaxed = true)
        val serverRepo = mockk<EnrolledServerRepository>(relaxed = true)

        val clientCertificate = ClientCertificate(
            certificateDer = byteArrayOf(4, 5, 6, 7),
            privateKeyAlias = "alias"
        )
        coEvery { certificateManager.loadOrCreate() } returns clientCertificate
        coEvery { certificateManager.pinServerFingerprint(any()) } returns Unit

        val expectedServerId = "5b7b1f6e-7a97-4bb6-9f3d-4f0dcd2e4d55"
        val handshakeCert = mockk<X509Certificate>()
        val handshakeDer = byteArrayOf(8, 8, 8, 8)
        every { handshakeCert.encoded } returns handshakeDer
        val fingerprint = VerificationCodeGenerator.computeFingerprint(handshakeDer)

        val directChannel = mockk<ManagedChannel>(relaxed = true)
        val response = EnrollResponse.newBuilder()
            .setSuccess(true)
            .setClientId("client-999")
            .build()

        val repository = object : GrpcEnrollmentRepository(
            context,
            certificateManager,
            mtlsFactory,
            relayFactory,
            serverRepo
        ) {
            var handled = false

            override suspend fun openEnrollmentChannel(
                host: String,
                port: Int,
                expectedFingerprint: String?
            ): EnrollmentChannel {
                return EnrollmentChannel(directChannel) { handshakeCert }
            }

            override suspend fun sendEnrollmentRequest(
                channel: ManagedChannel,
                request: com.handcontrol.grpc.EnrollRequest
            ): EnrollResponse {
                return response
            }

            override suspend fun handleSuccessfulEnrollment(
                channel: ManagedChannel,
                response: EnrollResponse,
                fingerprint: String,
                expectedServerId: String,
                allHosts: List<String>,
                port: Int,
                relayOptions: RelayEnrollmentOptions?
            ): EnrollmentResult {
                handled = true
                return EnrollmentResult.Success(response.clientId, expectedServerId)
            }

            override suspend fun shutdownEnrollmentChannel(channel: ManagedChannel) {
                // no-op for test
            }
        }

        val result = repository.enrollWithToken(
            hosts = listOf("192.168.1.50"),
            port = 50051,
            token = UUID.randomUUID().toString(),
            deviceName = "Unit Test",
            expectedCertFingerprint = fingerprint,
            expectedServerId = expectedServerId,
            validUntil = Instant.now().plusSeconds(120),
            relayOptions = null
        )

        assertTrue(result is EnrollmentResult.Success)
        assertTrue(repository.handled)
        coVerify(exactly = 0) {
            relayFactory.createChannelViaRelay(any(), any(), any(), any(), any(), any(), any(), any())
        }
    }

    @Test
    fun `direct enrollment propagates server error message`() = runTest {
        val context = mockk<Context>(relaxed = true)
        val certificateManager = mockk<ClientCertificateManager>()
        val mtlsFactory = mockk<MtlsGrpcChannelFactory>(relaxed = true)
        val relayFactory = mockk<RelayGrpcChannelFactory>(relaxed = true)
        val serverRepo = mockk<EnrolledServerRepository>(relaxed = true)

        val clientCertificate = ClientCertificate(
            certificateDer = byteArrayOf(10, 11, 12),
            privateKeyAlias = "alias"
        )
        coEvery { certificateManager.loadOrCreate() } returns clientCertificate
        coEvery { certificateManager.pinServerFingerprint(any()) } returns Unit

        val handshakeCert = mockk<X509Certificate>()
        val handshakeDer = byteArrayOf(13, 13, 13)
        every { handshakeCert.encoded } returns handshakeDer
        val fingerprint = VerificationCodeGenerator.computeFingerprint(handshakeDer)

        val directChannel = mockk<ManagedChannel>(relaxed = true)
        val errorResponse = EnrollResponse.newBuilder()
            .setSuccess(false)
            .setErrorMessage("server denied enrollment")
            .build()

        val repository = object : GrpcEnrollmentRepository(
            context,
            certificateManager,
            mtlsFactory,
            relayFactory,
            serverRepo
        ) {
            override suspend fun openEnrollmentChannel(
                host: String,
                port: Int,
                expectedFingerprint: String?
            ): EnrollmentChannel {
                return EnrollmentChannel(directChannel) { handshakeCert }
            }

            override suspend fun sendEnrollmentRequest(
                channel: ManagedChannel,
                request: com.handcontrol.grpc.EnrollRequest
            ): EnrollResponse {
                return errorResponse
            }

            override suspend fun shutdownEnrollmentChannel(channel: ManagedChannel) {
                // ignore
            }
        }

        val result = repository.enrollWithToken(
            hosts = listOf("192.168.1.60"),
            port = 50051,
            token = UUID.randomUUID().toString(),
            deviceName = "Unit Test",
            expectedCertFingerprint = fingerprint,
            expectedServerId = "d87d1ce3-7f6d-4480-a966-efa7440db28f",
            validUntil = Instant.now().plusSeconds(120),
            relayOptions = null
        )

        assertTrue(result is EnrollmentResult.Error)
        result as EnrollmentResult.Error
        assertEquals("server denied enrollment", result.message)
        coVerify(exactly = 0) {
            relayFactory.createChannelViaRelay(any(), any(), any(), any(), any(), any(), any(), any())
        }
    }

    @Test
    fun `relay enrollment fails fast when token expired`() = runTest {
        val context = mockk<Context>(relaxed = true)
        val certificateManager = mockk<ClientCertificateManager>()
        val mtlsFactory = mockk<MtlsGrpcChannelFactory>(relaxed = true)
        val relayFactory = mockk<RelayGrpcChannelFactory>(relaxed = true)
        val serverRepo = mockk<EnrolledServerRepository>(relaxed = true)

        val expectedFingerprint = "SHA256:deadbeef"
        coEvery { certificateManager.loadOrCreate() } returns ClientCertificate(byteArrayOf(), "alias")

        val repository = object : GrpcEnrollmentRepository(
            context,
            certificateManager,
            mtlsFactory,
            relayFactory,
            serverRepo
        ) {
            override suspend fun openEnrollmentChannel(
                host: String,
                port: Int,
                expectedFingerprint: String?
            ): EnrollmentChannel {
                throw AssertionError("Direct enrollment should not be attempted when expired")
            }
        }

        val options = RelayEnrollmentOptions(
            relayUrl = "wss://relay.example.com",
            relayToken = "relay-token",
            relayRequired = true,
            allowSelfSignedTls = false,
            pinnedCertSha256 = null
        )

        val result = repository.enrollWithToken(
            hosts = listOf("10.0.0.1"),
            port = 50051,
            token = UUID.randomUUID().toString(),
            deviceName = "Expired Token Test",
            expectedCertFingerprint = expectedFingerprint,
            expectedServerId = "11111111-2222-3333-4444-555555555555",
            validUntil = Instant.now().minusSeconds(30),
            relayOptions = options
        )

        assertTrue(result is EnrollmentResult.Error)
        result as EnrollmentResult.Error
        assertTrue(result.message.contains("expired", ignoreCase = true))
        coVerify(exactly = 0) {
            relayFactory.createChannelViaRelay(any(), any(), any(), any(), any(), any(), any(), any())
        }
    }

    @Test
    fun `relay enrollment surfaces connection failure message`() = runTest {
        val context = mockk<Context>(relaxed = true)
        val certificateManager = mockk<ClientCertificateManager>()
        val mtlsFactory = mockk<MtlsGrpcChannelFactory>(relaxed = true)
        val relayFactory = mockk<RelayGrpcChannelFactory>()
        val serverRepo = mockk<EnrolledServerRepository>(relaxed = true)

        coEvery { certificateManager.loadOrCreate() } returns ClientCertificate(byteArrayOf(), "alias")
        coEvery {
            relayFactory.createChannelViaRelay(any(), any(), any(), any(), any(), any(), any(), any())
        } throws RelayConnectionException(shortMessage = "relay failure", userMessage = "relay unavailable")

        val repository = object : GrpcEnrollmentRepository(
            context,
            certificateManager,
            mtlsFactory,
            relayFactory,
            serverRepo
        ) {
            override suspend fun openEnrollmentChannel(
                host: String,
                port: Int,
                expectedFingerprint: String?
            ): EnrollmentChannel {
                throw IOException("Direct path blocked for test")
            }
        }

        val result = repository.enrollWithToken(
            hosts = listOf("203.0.113.10"),
            port = 50051,
            token = UUID.randomUUID().toString(),
            deviceName = "Relay Failure Test",
            expectedCertFingerprint = "SHA256:feedface",
            expectedServerId = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee",
            validUntil = Instant.now().plusSeconds(300),
            relayOptions = RelayEnrollmentOptions(
                relayUrl = "wss://relay.example.com",
                relayToken = "relay-token",
                relayRequired = true,
                allowSelfSignedTls = false,
                pinnedCertSha256 = null
            )
        )

        assertTrue(result is EnrollmentResult.Error)
        result as EnrollmentResult.Error
        assertTrue(result.message.contains("Relay connection failed", ignoreCase = true))
        assertTrue(result.message.contains("relay unavailable", ignoreCase = true))
    }

    @Test
    fun `compute default relay authority wraps IPv6 host`() {
        val context = mockk<Context>(relaxed = true)
        val certificateManager = mockk<ClientCertificateManager>(relaxed = true)
        val mtlsFactory = mockk<MtlsGrpcChannelFactory>(relaxed = true)
        val relayFactory = mockk<RelayGrpcChannelFactory>(relaxed = true)
        val serverRepo = mockk<EnrolledServerRepository>(relaxed = true)

        val repository = object : GrpcEnrollmentRepository(
            context,
            certificateManager,
            mtlsFactory,
            relayFactory,
            serverRepo
        ) {
            fun authority(hosts: List<String>, port: Int): String = computeDefaultRelayAuthority(hosts, port)
        }

        val ipv6Authority = repository.authority(listOf("2001:db8::1"), 50051)
        assertEquals("[2001:db8::1]:50051", ipv6Authority)

        val hostnameAuthority = repository.authority(listOf("server.local"), 443)
        assertEquals("server.local:443", hostnameAuthority)
    }
}

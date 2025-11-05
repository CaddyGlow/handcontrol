@file:OptIn(kotlinx.coroutines.ExperimentalCoroutinesApi::class)

package com.handcontrol.data.enrollment

import android.content.Context
import com.google.protobuf.ByteString
import com.handcontrol.core.network.MtlsGrpcChannelFactory
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
}

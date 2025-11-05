@file:OptIn(kotlinx.coroutines.ExperimentalCoroutinesApi::class)

package com.handcontrol.feature.enrollment

import com.handcontrol.core.security.ClientCertificate
import com.handcontrol.data.database.EnrolledServerRepository
import com.handcontrol.data.enrollment.EnrollmentRepository
import com.handcontrol.data.enrollment.EnrollmentResult
import io.mockk.coEvery
import io.mockk.mockk
import java.time.Instant
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.test.setMain
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test

class EnrollmentViewModelTest {

    private val dispatcher = StandardTestDispatcher()

    @Before
    fun setup() {
        Dispatchers.setMain(dispatcher)
    }

    @After
    fun tearDown() {
        Dispatchers.resetMain()
    }

    @Test
    fun `expired QR payload produces error and skips enrollment`() = runTest(dispatcher) {
        val fakeRepository = FakeEnrollmentRepository()
        val enrolledRepository = mockk<EnrolledServerRepository>()
        coEvery { enrolledRepository.getServerByHostAndPort(any(), any()) } returns null

        val viewModel = EnrollmentViewModel(fakeRepository, enrolledRepository)
        val expired = Instant.now().minusSeconds(5)

        viewModel.enrollWithQrCode(
            hosts = listOf("10.0.0.1"),
            port = 50051,
            token = "test-token",
            certFingerprint = "SHA256:deadbeef",
            serverId = "server-1234",
            validUntil = expired
        )

        advanceUntilIdle()

        val state = viewModel.uiState.value
        assertTrue(state is EnrollmentUiState.Error)
        assertTrue((state as EnrollmentUiState.Error).message.contains("expired", ignoreCase = true))
        assertEquals(0, fakeRepository.enrollCallCount)
    }

    private class FakeEnrollmentRepository : EnrollmentRepository {
        var enrollCallCount = 0

        override suspend fun enrollWithToken(
            hosts: List<String>,
            port: Int,
            token: String,
            deviceName: String,
            expectedCertFingerprint: String,
            expectedServerId: String,
            validUntil: Instant?
        ): EnrollmentResult {
            enrollCallCount += 1
            return EnrollmentResult.Success(
                clientId = "client-123",
                serverId = expectedServerId
            )
        }

        override suspend fun requestApproval(
            host: String,
            port: Int,
            deviceName: String,
            deviceModel: String?,
            serverId: String?
        ): EnrollmentResult {
            throw UnsupportedOperationException("Not needed for test")
        }

        override suspend fun pollApprovalStatus(
            host: String,
            port: Int,
            requestId: String
        ): EnrollmentResult {
            throw UnsupportedOperationException("Not needed for test")
        }

        override suspend fun activeClientCertificate(): ClientCertificate? = null
    }
}

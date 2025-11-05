package com.handcontrol.data.enrollment

import com.handcontrol.core.security.ClientCertificate
import java.time.Instant

sealed interface EnrollmentResult {
    data class Success(val clientId: String, val serverId: String) : EnrollmentResult
    data class Pending(val requestId: String, val timeoutSeconds: Int, val verificationCode: String) : EnrollmentResult
    data class Error(val message: String) : EnrollmentResult
}

interface EnrollmentRepository {
    suspend fun enrollWithToken(
        hosts: List<String>,
        port: Int,
        token: String,
        deviceName: String,
        expectedCertFingerprint: String,
        expectedServerId: String,
        validUntil: Instant?
    ): EnrollmentResult

    suspend fun requestApproval(
        host: String,
        port: Int,
        deviceName: String,
        deviceModel: String?,
        serverId: String?
    ): EnrollmentResult

    suspend fun pollApprovalStatus(
        host: String,
        port: Int,
        requestId: String
    ): EnrollmentResult

    suspend fun activeClientCertificate(): ClientCertificate?
}

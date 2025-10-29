package com.handcontrol.data.enrollment

import com.handcontrol.core.security.ClientCertificate

sealed interface EnrollmentResult {
    data class Success(val clientId: String) : EnrollmentResult
    data class Pending(val requestId: String, val timeoutSeconds: Int) : EnrollmentResult
    data class Error(val message: String) : EnrollmentResult
}

interface EnrollmentRepository {
    suspend fun enrollWithToken(
        host: String,
        port: Int,
        token: String,
        deviceName: String
    ): EnrollmentResult

    suspend fun requestApproval(
        host: String,
        port: Int,
        deviceName: String,
        deviceModel: String?
    ): EnrollmentResult

    suspend fun pollApprovalStatus(
        host: String,
        port: Int,
        requestId: String
    ): EnrollmentResult

    suspend fun activeClientCertificate(): ClientCertificate?
}

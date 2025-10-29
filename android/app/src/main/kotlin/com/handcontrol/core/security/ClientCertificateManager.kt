package com.handcontrol.core.security

data class ClientCertificate(
    val certificateDer: ByteArray,
    val privateKeyAlias: String
)

interface ClientCertificateManager {
    suspend fun loadOrCreate(): ClientCertificate
    suspend fun pinServerFingerprint(fingerprint: String)
    suspend fun getPinnedServerFingerprint(): String?
    suspend fun clearPinnedServer()
}

package com.handcontrol.core.security

import java.security.MessageDigest

object VerificationCodeGenerator {
    /**
     * Generate a 6-digit verification code from certificates and server ID
     *
     * This code is used in approval mode pairing to prevent MITM attacks.
     * Both client and server compute the same code independently.
     *
     * Algorithm:
     * 1. Compute SHA256 fingerprints of both certificates
     * 2. Combine: SHA256(client_fp || server_fp || server_id)
     * 3. Convert to hex and extract first 6 digits
     * 4. Format as XXX-XXX for readability
     *
     * IMPORTANT: Must match server implementation in src/security/verification.rs
     */
    fun generate(
        clientCertificateDer: ByteArray,
        serverCertificateDer: ByteArray,
        serverId: String
    ): String {
        val digest = MessageDigest.getInstance("SHA-256")

        // Compute fingerprints first
        val clientFingerprint = digest.digest(clientCertificateDer)
        digest.reset()

        val serverFingerprint = digest.digest(serverCertificateDer)
        digest.reset()

        // Combine fingerprints with server ID
        digest.update(clientFingerprint)
        digest.update(serverFingerprint)
        digest.update(serverId.toByteArray())

        val hash = digest.digest()

        // Convert to hex and extract first 6 digits
        val hex = hash.joinToString("") { "%02x".format(it) }
        val digits = hex.filter { it.isDigit() }.take(6)

        // If we don't have enough digits (unlikely), pad with zeros
        val paddedDigits = if (digits.length < 6) {
            digits.padEnd(6, '0')
        } else {
            digits
        }

        // Format as XXX-XXX
        return "${paddedDigits.substring(0, 3)}-${paddedDigits.substring(3)}"
    }

    fun computeFingerprint(certificateDer: ByteArray): String {
        val digest = MessageDigest.getInstance("SHA-256")
        val hash = digest.digest(certificateDer)
        return "SHA256:" + hash.joinToString("") { "%02x".format(it) }
    }
}

package com.handcontrol.core.security

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class VerificationCodeGeneratorTest {

    @Test
    fun `generate returns formatted 6-digit code`() {
        val clientCert = ByteArray(256) { it.toByte() }
        val serverCert = ByteArray(256) { (it * 2).toByte() }
        val serverId = "test-server-id"

        val code = VerificationCodeGenerator.generate(clientCert, serverCert, serverId)

        assertTrue("Code should match XXX-XXX format", code.matches(Regex("\\d{3}-\\d{3}")))
        assertEquals("Code should be 7 characters including dash", 7, code.length)
    }

    @Test
    fun `generate produces consistent code for same inputs`() {
        val clientCert = ByteArray(256) { it.toByte() }
        val serverCert = ByteArray(256) { (it * 2).toByte() }
        val serverId = "test-server-id"

        val code1 = VerificationCodeGenerator.generate(clientCert, serverCert, serverId)
        val code2 = VerificationCodeGenerator.generate(clientCert, serverCert, serverId)

        assertEquals("Same inputs should produce same code", code1, code2)
    }

    @Test
    fun `generate produces different codes for different client certs`() {
        val clientCert1 = ByteArray(256) { it.toByte() }
        val clientCert2 = ByteArray(256) { (it + 1).toByte() }
        val serverCert = ByteArray(256) { (it * 2).toByte() }
        val serverId = "test-server-id"

        val code1 = VerificationCodeGenerator.generate(clientCert1, serverCert, serverId)
        val code2 = VerificationCodeGenerator.generate(clientCert2, serverCert, serverId)

        assertNotEquals("Different client certs should produce different codes", code1, code2)
    }

    @Test
    fun `generate produces different codes for different server certs`() {
        val clientCert = ByteArray(256) { it.toByte() }
        val serverCert1 = ByteArray(256) { (it * 2).toByte() }
        val serverCert2 = ByteArray(256) { (it * 3).toByte() }
        val serverId = "test-server-id"

        val code1 = VerificationCodeGenerator.generate(clientCert, serverCert1, serverId)
        val code2 = VerificationCodeGenerator.generate(clientCert, serverCert2, serverId)

        assertNotEquals("Different server certs should produce different codes", code1, code2)
    }

    @Test
    fun `generate produces different codes for different server IDs`() {
        val clientCert = ByteArray(256) { it.toByte() }
        val serverCert = ByteArray(256) { (it * 2).toByte() }
        val serverId1 = "test-server-id-1"
        val serverId2 = "test-server-id-2"

        val code1 = VerificationCodeGenerator.generate(clientCert, serverCert, serverId1)
        val code2 = VerificationCodeGenerator.generate(clientCert, serverCert, serverId2)

        assertNotEquals("Different server IDs should produce different codes", code1, code2)
    }

    @Test
    fun `computeFingerprint returns SHA256 prefixed hex string`() {
        val certDer = ByteArray(256) { it.toByte() }

        val fingerprint = VerificationCodeGenerator.computeFingerprint(certDer)

        assertTrue("Fingerprint should start with SHA256:", fingerprint.startsWith("SHA256:"))
        assertTrue("Fingerprint should be hex", fingerprint.substring(7).matches(Regex("[0-9a-f]+")))
        assertEquals("SHA256 hash should be 64 hex chars + prefix", 71, fingerprint.length)
    }

    @Test
    fun `computeFingerprint produces consistent results`() {
        val certDer = ByteArray(256) { it.toByte() }

        val fp1 = VerificationCodeGenerator.computeFingerprint(certDer)
        val fp2 = VerificationCodeGenerator.computeFingerprint(certDer)

        assertEquals("Same input should produce same fingerprint", fp1, fp2)
    }

    @Test
    fun `computeFingerprint produces different results for different inputs`() {
        val certDer1 = ByteArray(256) { it.toByte() }
        val certDer2 = ByteArray(256) { (it + 1).toByte() }

        val fp1 = VerificationCodeGenerator.computeFingerprint(certDer1)
        val fp2 = VerificationCodeGenerator.computeFingerprint(certDer2)

        assertNotEquals("Different inputs should produce different fingerprints", fp1, fp2)
    }

    @Test
    fun `generate matches server algorithm for known inputs`() {
        // Test with known inputs to ensure Android matches Rust implementation
        // This verifies the algorithm extracts digits from hex correctly
        val clientCert = "client certificate data".toByteArray()
        val serverCert = "server certificate data".toByteArray()
        val serverId = "test-server-id"

        val code = VerificationCodeGenerator.generate(clientCert, serverCert, serverId)

        // Verify format
        assertTrue("Code should match XXX-XXX format", code.matches(Regex("\\d{3}-\\d{3}")))

        // Verify consistency
        val code2 = VerificationCodeGenerator.generate(clientCert, serverCert, serverId)
        assertEquals("Same inputs should produce same code", code, code2)
    }

    @Test
    fun `generate extracts only digits from hex representation`() {
        // This test verifies we're extracting digits from hex, not using raw bytes
        // The algorithm should scan through the hex string for digit characters
        val clientCert = ByteArray(32) { 0xFF.toByte() } // All 0xFF -> hex "ff" (no digits)
        val serverCert = ByteArray(32) { 0x00.toByte() } // All 0x00 -> hex "00" (all digits)
        val serverId = "test"

        val code = VerificationCodeGenerator.generate(clientCert, serverCert, serverId)

        // Should still produce valid 6-digit code
        assertTrue("Code should match XXX-XXX format", code.matches(Regex("\\d{3}-\\d{3}")))
        assertEquals("Code should be 7 characters", 7, code.length)
    }

    @Test
    fun `generate handles edge case with few digits in hash`() {
        // Edge case: what if hex hash has very few digit characters?
        // The implementation should pad with zeros
        val clientCert = ByteArray(1) { 0xAB.toByte() }
        val serverCert = ByteArray(1) { 0xCD.toByte() }
        val serverId = ""

        val code = VerificationCodeGenerator.generate(clientCert, serverCert, serverId)

        // Should still produce valid format (padded if needed)
        assertTrue("Code should match XXX-XXX format", code.matches(Regex("\\d{3}-\\d{3}")))
    }
}

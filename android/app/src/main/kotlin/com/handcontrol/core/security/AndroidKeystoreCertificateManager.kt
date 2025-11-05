package com.handcontrol.core.security

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import com.handcontrol.data.database.EnrolledServerRepository
import dagger.hilt.android.qualifiers.ApplicationContext
import timber.log.Timber
import java.math.BigInteger
import java.security.GeneralSecurityException
import java.security.KeyPairGenerator
import java.security.KeyStore
import java.security.Signature
import java.security.cert.X509Certificate
import java.util.Date
import javax.inject.Inject
import javax.inject.Singleton
import javax.security.auth.x500.X500Principal

@Singleton
class AndroidKeystoreCertificateManager @Inject constructor(
    @ApplicationContext private val context: Context,
    private val enrolledServerRepository: EnrolledServerRepository
) : ClientCertificateManager {

    private val keyStore: KeyStore = KeyStore.getInstance(ANDROID_KEYSTORE).apply {
        load(null)
    }

    override suspend fun loadOrCreate(): ClientCertificate {
        if (keyStore.containsAlias(CLIENT_KEY_ALIAS)) {
            Timber.d("Loading existing client certificate")
            if (!isKeyUsable(CLIENT_KEY_ALIAS)) {
                Timber.w("Client key alias %s is not usable, regenerating", CLIENT_KEY_ALIAS)
                keyStore.deleteEntry(CLIENT_KEY_ALIAS)
            } else {
                val certificate = keyStore.getCertificate(CLIENT_KEY_ALIAS) as X509Certificate
                return ClientCertificate(
                    certificateDer = certificate.encoded,
                    privateKeyAlias = CLIENT_KEY_ALIAS
                )
            }
        }

        Timber.i("Generating new client certificate")
        return generateClientCertificate()
    }

    override suspend fun pinServerFingerprint(fingerprint: String) {
        Timber.i("Pinning server certificate fingerprint: $fingerprint")
        // Fingerprints are now stored in Room database per server
        // This method is kept for interface compatibility but actual storage
        // happens in EnrolledServerRepository during enrollment
    }

    override suspend fun getPinnedServerFingerprint(): String? {
        // Return fingerprint from last connected server for backwards compatibility
        val lastServer = enrolledServerRepository.getLastConnectedServer()
        return lastServer?.certFingerprint
    }

    override suspend fun clearPinnedServer() {
        Timber.i("Clearing pinned server certificate")
        // This would remove all enrolled servers - not recommended
        // Keeping as no-op for safety, individual servers should be removed via repository
    }

    private fun generateClientCertificate(): ClientCertificate {
        val keyPairGenerator = KeyPairGenerator.getInstance(
            KeyProperties.KEY_ALGORITHM_EC,
            ANDROID_KEYSTORE
        )

        val spec = KeyGenParameterSpec.Builder(
            CLIENT_KEY_ALIAS,
            KeyProperties.PURPOSE_SIGN or KeyProperties.PURPOSE_VERIFY
        )
            .setAlgorithmParameterSpec(java.security.spec.ECGenParameterSpec("secp256r1"))
            .setDigests(
                KeyProperties.DIGEST_SHA256,
                KeyProperties.DIGEST_SHA384,
                KeyProperties.DIGEST_SHA512,
                KeyProperties.DIGEST_NONE
            )
            .setUserAuthenticationRequired(false)
            .setCertificateSubject(X500Principal("CN=HandControl Client"))
            .setCertificateSerialNumber(BigInteger.valueOf(System.currentTimeMillis()))
            .setCertificateNotBefore(Date())
            .setCertificateNotAfter(Date(System.currentTimeMillis() + CERT_VALIDITY_MS))
            .build()

        keyPairGenerator.initialize(spec)
        keyPairGenerator.generateKeyPair()

        val certificate = keyStore.getCertificate(CLIENT_KEY_ALIAS) as X509Certificate

        return ClientCertificate(
            certificateDer = certificate.encoded,
            privateKeyAlias = CLIENT_KEY_ALIAS
        )
    }

    private fun isKeyUsable(alias: String): Boolean {
        return try {
            val entry = keyStore.getEntry(alias, null) as? KeyStore.PrivateKeyEntry ?: return false
            val algorithms = listOf("SHA256withECDSA", "NONEwithECDSA")

            for (algorithm in algorithms) {
                val signature = Signature.getInstance(algorithm)
                signature.initSign(entry.privateKey)
                signature.update(byteArrayOf(0x01))
                signature.sign()
            }

            true
        } catch (e: GeneralSecurityException) {
            Timber.w(e, "Client key alias %s failed usability check", alias)
            false
        }
    }

    companion object {
        private const val ANDROID_KEYSTORE = "AndroidKeyStore"
        private const val CLIENT_KEY_ALIAS = "handcontrol_client_key"
        private const val CERT_VALIDITY_MS = 10L * 365 * 24 * 60 * 60 * 1000 // 10 years
    }
}

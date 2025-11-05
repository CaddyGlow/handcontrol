package com.handcontrol.core.network

import com.handcontrol.core.security.ClientCertificateManager
import com.handcontrol.core.security.VerificationCodeGenerator
import timber.log.Timber
import java.security.KeyStore
import java.security.cert.X509Certificate
import java.util.Locale
import javax.net.ssl.KeyManagerFactory
import javax.net.ssl.SSLContext
import javax.net.ssl.X509TrustManager

object MtlsSslContextFactory {
    suspend fun createSslContext(
        certificateManager: ClientCertificateManager,
        expectedFingerprint: String?,
        onServerCertificate: ((X509Certificate) -> Unit)? = null
    ): SSLContext {
        certificateManager.loadOrCreate()
        val normalizedExpectedFingerprint = expectedFingerprint
            ?.takeIf { it.isNotBlank() }
            ?.removePrefix("SHA256:")
            ?.lowercase(Locale.US)

        val trustManager = object : X509TrustManager {
            override fun checkClientTrusted(chain: Array<out X509Certificate>?, authType: String?) {
                // Not used on client side
            }

            override fun checkServerTrusted(chain: Array<out X509Certificate>?, authType: String?) {
                if (chain.isNullOrEmpty()) {
                    throw javax.net.ssl.SSLException("Server certificate chain is empty")
                }

                val serverCert = chain[0]
                onServerCertificate?.invoke(serverCert)

                val currentFingerprint = VerificationCodeGenerator.computeFingerprint(
                    serverCert.encoded
                )
                val normalizedCurrent = currentFingerprint
                    .removePrefix("SHA256:")
                    .lowercase(Locale.US)

                if (normalizedExpectedFingerprint != null) {
                    if (normalizedCurrent != normalizedExpectedFingerprint) {
                        Timber.e("Server certificate fingerprint mismatch!")
                        Timber.e("Expected: ${expectedFingerprint ?: "unknown"}")
                        Timber.e("Got: $currentFingerprint")
                        throw javax.net.ssl.SSLException(
                            "Server certificate fingerprint does not match expected value"
                        )
                    }
                    Timber.d("Server certificate fingerprint verified")
                } else {
                    Timber.w("No expected server certificate fingerprint provided; skipping pin validation")
                }

                try {
                    serverCert.checkValidity()
                } catch (e: Exception) {
                    throw javax.net.ssl.SSLException("Server certificate is not valid", e)
                }

                Timber.d("Server certificate validation passed")
            }

            override fun getAcceptedIssuers(): Array<X509Certificate> = emptyArray()
        }

        val androidKeyStore = KeyStore.getInstance("AndroidKeyStore").apply {
            load(null)
        }

        val keyManagerFactory = KeyManagerFactory.getInstance(
            KeyManagerFactory.getDefaultAlgorithm()
        )
        keyManagerFactory.init(androidKeyStore, null)

        return SSLContext.getInstance("TLS").apply {
            init(keyManagerFactory.keyManagers, arrayOf(trustManager), null)
        }
    }
}

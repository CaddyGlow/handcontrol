package com.handcontrol.core.network

import com.handcontrol.core.security.ClientCertificateManager
import com.handcontrol.core.security.VerificationCodeGenerator
import timber.log.Timber
import java.net.Socket
import java.security.KeyStore
import java.security.Principal
import java.security.cert.X509Certificate
import java.util.Locale
import javax.net.ssl.KeyManager
import javax.net.ssl.KeyManagerFactory
import javax.net.ssl.SSLEngine
import javax.net.ssl.SSLContext
import javax.net.ssl.X509ExtendedKeyManager
import javax.net.ssl.X509TrustManager

object MtlsSslContextFactory {
    suspend fun createSslContext(
        certificateManager: ClientCertificateManager,
        expectedFingerprint: String?,
        onServerCertificate: ((X509Certificate) -> Unit)? = null,
        includeClientCertificate: Boolean = true
    ): SSLContext {
        val trustManager = buildTrustManager(expectedFingerprint, onServerCertificate)

        val keyManagers: Array<KeyManager>? = if (includeClientCertificate) {
            val clientCertificate = certificateManager.loadOrCreate()

            val androidKeyStore = KeyStore.getInstance("AndroidKeyStore").apply {
                load(null)
            }

            val keyManagerFactory = KeyManagerFactory.getInstance(
                KeyManagerFactory.getDefaultAlgorithm()
            )
            keyManagerFactory.init(androidKeyStore, null)

            wrapKeyManagers(
                clientCertificate.privateKeyAlias,
                keyManagerFactory.keyManagers
            )
        } else {
            null
        }

        return SSLContext.getInstance("TLS").apply {
            init(keyManagers, arrayOf(trustManager), null)
        }
    }

    private fun wrapKeyManagers(
        preferredAlias: String,
        keyManagers: Array<KeyManager>
    ): Array<KeyManager> {
        return keyManagers.map { manager ->
            if (manager is X509ExtendedKeyManager) {
                AliasForcingKeyManager(preferredAlias, manager)
            } else {
                manager
            }
        }.toTypedArray()
    }

    private fun buildTrustManager(
        expectedFingerprint: String?,
        onServerCertificate: ((X509Certificate) -> Unit)?
    ): X509TrustManager {
        val normalizedExpectedFingerprint = expectedFingerprint
            ?.takeIf { it.isNotBlank() }
            ?.removePrefix("SHA256:")
            ?.lowercase(Locale.US)

        return object : X509TrustManager {
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
    }

    private class AliasForcingKeyManager(
        private val preferredAlias: String,
        private val delegate: X509ExtendedKeyManager
    ) : X509ExtendedKeyManager() {

        override fun chooseClientAlias(
            keyType: Array<out String>?,
            issuers: Array<out Principal>?,
            socket: Socket?
        ): String? = selectAlias(keyType) ?: delegate.chooseClientAlias(keyType, issuers, socket)

        override fun chooseEngineClientAlias(
            keyType: Array<out String>?,
            issuers: Array<out Principal>?,
            engine: SSLEngine?
        ): String? = selectAlias(keyType)
            ?: delegate.chooseEngineClientAlias(keyType, issuers, engine)

        override fun getClientAliases(
            keyType: String?,
            issuers: Array<out Principal>?
        ): Array<String>? = delegate.getClientAliases(keyType, issuers)

        override fun getServerAliases(
            keyType: String?,
            issuers: Array<out Principal>?
        ): Array<String>? = delegate.getServerAliases(keyType, issuers)

        override fun chooseServerAlias(
            keyType: String?,
            issuers: Array<out Principal>?,
            socket: Socket?
        ): String? = delegate.chooseServerAlias(keyType, issuers, socket)

        override fun chooseEngineServerAlias(
            keyType: String?,
            issuers: Array<out Principal>?,
            engine: SSLEngine?
        ): String? = delegate.chooseEngineServerAlias(keyType, issuers, engine)

        private fun selectAlias(keyTypes: Array<out String>?): String? {
            val chain = delegate.getCertificateChain(preferredAlias) ?: return null
            if (chain.isEmpty()) {
                return null
            }

            val certificate = chain.first()
            val algorithm = certificate.publicKey.algorithm.uppercase(Locale.US)

            val matchesKeyType = keyTypes.isNullOrEmpty() || keyTypes.any { type ->
                val normalized = type.uppercase(Locale.US)
                normalized == algorithm ||
                    (normalized == "EC" && algorithm == "ECDSA") ||
                    (normalized == "ECDSA" && algorithm == "EC")
            }

            return if (matchesKeyType) {
                Timber.d("Using client certificate alias '%s' for mTLS handshake", preferredAlias)
                Timber.v(
                    "Client cert alias '%s' chain length=%d, keyTypes=%s",
                    preferredAlias,
                    chain.size,
                    keyTypes?.joinToString() ?: "any"
                )
                preferredAlias
            } else {
                null
            }
        }

        override fun getCertificateChain(alias: String?): Array<X509Certificate>? {
            val chain = delegate.getCertificateChain(alias)
            if (alias == preferredAlias) {
                Timber.v(
                    "Providing certificate chain for alias '%s' (length=%d)",
                    alias,
                    chain?.size ?: 0
                )
            }
            return chain
        }

        override fun getPrivateKey(alias: String?) = delegate.getPrivateKey(alias)?.also {
            if (alias == preferredAlias) {
                Timber.v("Providing private key for alias '%s'", alias)
            }
        }
    }
}

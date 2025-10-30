package com.handcontrol.core.network

import com.handcontrol.core.security.ClientCertificateManager
import com.handcontrol.core.security.VerificationCodeGenerator
import io.grpc.ManagedChannel
import io.grpc.okhttp.OkHttpChannelBuilder
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import timber.log.Timber
import java.io.ByteArrayInputStream
import java.security.KeyStore
import java.security.cert.Certificate
import java.security.cert.CertificateFactory
import java.security.cert.X509Certificate
import java.util.concurrent.TimeUnit
import javax.inject.Inject
import javax.inject.Singleton
import javax.net.ssl.KeyManagerFactory
import javax.net.ssl.SSLContext
import javax.net.ssl.X509TrustManager

@Singleton
class MtlsGrpcChannelFactory @Inject constructor(
    private val certificateManager: ClientCertificateManager
) : GrpcChannelFactory {

    private val activeChannels = mutableSetOf<ManagedChannel>()
    private var lastServerCertificate: X509Certificate? = null

    override suspend fun createChannel(host: String, port: Int): ManagedChannel = withContext(Dispatchers.IO) {
        // Remove brackets from IPv6 addresses if present (forAddress handles raw IPv6)
        val cleanHost = host.trimStart('[').trimEnd(']')
        val isIpv6 = cleanHost.contains(':')

        Timber.i("Creating mTLS gRPC channel to ${if (isIpv6) "[$cleanHost]" else cleanHost}:$port (IPv6: $isIpv6)")

        val clientCert = certificateManager.loadOrCreate()
        val trustManager = createCapturingTrustManager()
        val sslContext = createMtlsSslContext(clientCert.privateKeyAlias, trustManager)

        val builder = OkHttpChannelBuilder
            .forAddress(cleanHost, port)
            .sslSocketFactory(sslContext.socketFactory)
            .hostnameVerifier { _, _ -> true }
            .keepAliveTime(30, TimeUnit.SECONDS)
            .keepAliveTimeout(10, TimeUnit.SECONDS)
            .keepAliveWithoutCalls(true)

        // For IP addresses (both IPv4 and IPv6), we need to override authority with a dummy hostname
        // because SNI (Server Name Indication) requires a hostname, not an IP address
        val isIpAddress = isIpv6 || cleanHost.matches(Regex("^\\d+\\.\\d+\\.\\d+\\.\\d+$"))
        if (isIpAddress) {
            Timber.d("IP address detected, using overrideAuthority to bypass SNI hostname requirement")
            builder.overrideAuthority("handcontrol.local:$port")
        }

        val channel = builder.build()

        activeChannels.add(channel)
        Timber.d("gRPC channel created successfully for $cleanHost")
        channel
    }

    override suspend fun shutdownChannel(channel: ManagedChannel) = withContext(Dispatchers.IO) {
        Timber.i("Shutting down gRPC channel")
        channel.shutdown()

        try {
            if (!channel.awaitTermination(5, TimeUnit.SECONDS)) {
                Timber.w("Channel did not terminate gracefully, forcing shutdown")
                channel.shutdownNow()
                channel.awaitTermination(2, TimeUnit.SECONDS)
            }
        } catch (e: InterruptedException) {
            Timber.e(e, "Interrupted while shutting down channel")
            channel.shutdownNow()
            Thread.currentThread().interrupt()
        } finally {
            activeChannels.remove(channel)
        }
    }

    /**
     * Get the last server certificate encountered during TLS handshake.
     * This is needed for verification code generation in approval mode.
     */
    suspend fun getLastServerCertificate(): X509Certificate? {
        return lastServerCertificate
    }

    private suspend fun createMtlsSslContext(
        clientKeyAlias: String,
        trustManager: X509TrustManager
    ): SSLContext {
        val clientCert = certificateManager.loadOrCreate()

        // Load Android Keystore
        val androidKeyStore = KeyStore.getInstance("AndroidKeyStore").apply {
            load(null)
        }

        // Create key manager with client certificate
        val keyManagerFactory = KeyManagerFactory.getInstance(
            KeyManagerFactory.getDefaultAlgorithm()
        )
        keyManagerFactory.init(androidKeyStore, null)

        // Create SSL context
        val sslContext = SSLContext.getInstance("TLS")
        sslContext.init(
            keyManagerFactory.keyManagers,
            arrayOf(trustManager),
            null
        )

        return sslContext
    }

    private suspend fun createCapturingTrustManager(): X509TrustManager {
        val pinnedFingerprint = certificateManager.getPinnedServerFingerprint()

        return object : X509TrustManager {
            override fun checkClientTrusted(chain: Array<out X509Certificate>?, authType: String?) {
                // Not used on client side
            }

            override fun checkServerTrusted(chain: Array<out X509Certificate>?, authType: String?) {
                if (chain == null || chain.isEmpty()) {
                    throw javax.net.ssl.SSLException("Server certificate chain is empty")
                }

                val serverCert = chain[0]
                lastServerCertificate = serverCert

                // If we have a pinned fingerprint, validate it
                if (pinnedFingerprint != null) {
                    val currentFingerprint = VerificationCodeGenerator.computeFingerprint(
                        serverCert.encoded
                    )

                    if (currentFingerprint != pinnedFingerprint) {
                        Timber.e("Server certificate fingerprint mismatch!")
                        Timber.e("Expected: $pinnedFingerprint")
                        Timber.e("Got: $currentFingerprint")
                        throw javax.net.ssl.SSLException(
                            "Server certificate fingerprint does not match pinned value"
                        )
                    }
                    Timber.d("Server certificate fingerprint verified")
                } else {
                    Timber.w("No pinned server certificate - first connection")
                }

                // Validate certificate is not expired
                try {
                    serverCert.checkValidity()
                } catch (e: Exception) {
                    throw javax.net.ssl.SSLException("Server certificate is not valid", e)
                }

                Timber.d("Server certificate validation passed")
            }

            override fun getAcceptedIssuers(): Array<X509Certificate> {
                return arrayOf()
            }
        }
    }

    suspend fun shutdown() = withContext(Dispatchers.IO) {
        Timber.i("Shutting down all gRPC channels")
        activeChannels.toList().forEach { channel ->
            shutdownChannel(channel)
        }
    }
}

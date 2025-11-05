package com.handcontrol.core.network

import com.handcontrol.core.network.MtlsSslContextFactory
import com.handcontrol.core.security.ClientCertificateManager
import io.grpc.ManagedChannel
import io.grpc.okhttp.OkHttpChannelBuilder
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import timber.log.Timber
import java.security.cert.X509Certificate
import java.util.Collections
import java.util.WeakHashMap
import java.util.concurrent.atomic.AtomicReference
import java.util.concurrent.TimeUnit
import javax.inject.Inject
import javax.inject.Singleton

@Singleton
class MtlsGrpcChannelFactory @Inject constructor(
    private val certificateManager: ClientCertificateManager
) : GrpcChannelFactory {

    private val activeChannels = mutableSetOf<ManagedChannel>()
    private var lastServerCertificate: X509Certificate? = null
    private val channelCertificates =
        Collections.synchronizedMap(WeakHashMap<ManagedChannel, AtomicReference<X509Certificate?>>())

    override suspend fun createChannel(
        host: String,
        port: Int,
        expectedFingerprint: String?
    ): ManagedChannel = withContext(Dispatchers.IO) {
        // Remove brackets from IPv6 addresses if present (forAddress handles raw IPv6)
        val cleanHost = host.trimStart('[').trimEnd(']')
        val isIpv6 = cleanHost.contains(':')

        Timber.i("Creating mTLS gRPC channel to ${if (isIpv6) "[$cleanHost]" else cleanHost}:$port (IPv6: $isIpv6)")

        lastServerCertificate = null
        val certificateRef = AtomicReference<X509Certificate?>(null)
        val sslContext = MtlsSslContextFactory.createSslContext(
            certificateManager = certificateManager,
            expectedFingerprint = expectedFingerprint,
            onServerCertificate = { cert ->
                certificateRef.set(cert)
                lastServerCertificate = cert
            }
        )

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
        channelCertificates[channel] = certificateRef
        Timber.d("gRPC channel created successfully for $cleanHost")
        channel
    }

    suspend fun createChannel(host: String, port: Int): ManagedChannel =
        createChannel(host, port, null)

    override suspend fun shutdownChannel(channel: ManagedChannel) {
        withContext(Dispatchers.IO) {
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
            channelCertificates.remove(channel)
        }
    }

    /**
     * Immediately shuts down a channel without waiting for graceful termination.
     * Used when a connection attempt fails before the channel becomes usable.
     */
    suspend fun forceShutdownChannel(channel: ManagedChannel) = withContext(Dispatchers.IO) {
        Timber.d("Force shutting down gRPC channel")

        try {
            channel.shutdownNow()
        } catch (e: Exception) {
            Timber.d(e, "Ignoring force shutdown error")
        } finally {
            activeChannels.remove(channel)
        }
        channelCertificates.remove(channel)
    }

    /**
     * Get the last server certificate encountered during TLS handshake.
     * This is needed for verification code generation in approval mode.
     */
    suspend fun getLastServerCertificate(): X509Certificate? {
        return lastServerCertificate
    }

    fun getServerCertificate(channel: ManagedChannel): X509Certificate? {
        return channelCertificates[channel]?.get()
    }

    suspend fun shutdown() = withContext(Dispatchers.IO) {
        Timber.i("Shutting down all gRPC channels")
        activeChannels.toList().forEach { channel ->
            shutdownChannel(channel)
        }
        channelCertificates.clear()
    }
}

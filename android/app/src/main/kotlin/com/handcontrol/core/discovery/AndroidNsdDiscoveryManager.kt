package com.handcontrol.core.discovery

import android.content.Context
import android.net.nsd.NsdManager
import android.net.nsd.NsdServiceInfo
import android.os.Build
import dagger.hilt.android.qualifiers.ApplicationContext
import kotlinx.coroutines.channels.awaitClose
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.callbackFlow
import timber.log.Timber
import java.util.concurrent.Executor
import javax.inject.Inject
import javax.inject.Singleton

@Singleton
class AndroidNsdDiscoveryManager @Inject constructor(
    @ApplicationContext private val context: Context
) : NsdDiscoveryManager {

    private val nsdManager: NsdManager by lazy {
        context.getSystemService(Context.NSD_SERVICE) as NsdManager
    }

    private val _servers = MutableStateFlow<List<DiscoveredServer>>(emptyList())
    override val servers: StateFlow<List<DiscoveredServer>> = _servers.asStateFlow()

    private val discoveredServers = mutableMapOf<String, DiscoveredServer>()
    private var discoveryListener: NsdManager.DiscoveryListener? = null
    private var isDiscovering = false

    override suspend fun startDiscovery() {
        if (isDiscovering) {
            Timber.w("Discovery already in progress")
            return
        }

        Timber.i("Starting NSD discovery for _handcontrol._tcp")

        discoveryListener = object : NsdManager.DiscoveryListener {
            override fun onDiscoveryStarted(serviceType: String) {
                Timber.d("Discovery started: $serviceType")
                isDiscovering = true
            }

            override fun onServiceFound(serviceInfo: NsdServiceInfo) {
                Timber.d("Service found: ${serviceInfo.serviceName}")

                // Resolve the service to get host and port
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
                    val executor = context.mainExecutor
                    val callback = object : NsdManager.ServiceInfoCallback {
                        override fun onServiceInfoCallbackRegistrationFailed(errorCode: Int) {
                            Timber.w("Failed to register service info callback for ${serviceInfo.serviceName}: error $errorCode")
                        }

                        override fun onServiceUpdated(serviceInfo: NsdServiceInfo) {
                            Timber.i("Service resolved: ${serviceInfo.serviceName} at ${serviceInfo.hostAddresses}:${serviceInfo.port}")

                            val fingerprint = extractFingerprint(serviceInfo)
                            val serverId = extractServerId(serviceInfo)
                            val version = extractVersion(serviceInfo)

                            val host = serviceInfo.hostAddresses.firstOrNull()?.hostAddress
                                ?: serviceInfo.hostAddresses.firstOrNull()?.hostName
                                ?: "unknown"

                            val server = DiscoveredServer(
                                name = serviceInfo.serviceName,
                                host = host,
                                port = serviceInfo.port,
                                fingerprint = fingerprint,
                                serverId = serverId,
                                version = version
                            )

                            discoveredServers[serviceInfo.serviceName] = server
                            _servers.value = discoveredServers.values.toList()

                            Timber.d(
                                "Server added name=%s host=%s port=%d id=%s fingerprint=%s",
                                server.name,
                                server.host,
                                server.port,
                                server.serverId,
                                server.fingerprint
                            )
                        }

                        override fun onServiceLost() {
                            Timber.d("Service info callback lost for ${serviceInfo.serviceName}")
                        }

                        override fun onServiceInfoCallbackUnregistered() {
                            Timber.d("Service info callback unregistered for ${serviceInfo.serviceName}")
                        }
                    }
                    nsdManager.registerServiceInfoCallback(serviceInfo, executor, callback)
                } else {
                    @Suppress("DEPRECATION")
                    val resolveListener = object : NsdManager.ResolveListener {
                        override fun onResolveFailed(serviceInfo: NsdServiceInfo, errorCode: Int) {
                            Timber.w("Failed to resolve service ${serviceInfo.serviceName}: error $errorCode")
                        }

                        override fun onServiceResolved(serviceInfo: NsdServiceInfo) {
                            Timber.i("Service resolved: ${serviceInfo.serviceName} at ${serviceInfo.host}:${serviceInfo.port}")

                            val fingerprint = extractFingerprint(serviceInfo)
                            val serverId = extractServerId(serviceInfo)
                            val version = extractVersion(serviceInfo)

                            @Suppress("DEPRECATION")
                            val host = serviceInfo.host?.hostAddress ?: serviceInfo.host?.hostName ?: "unknown"

                            val server = DiscoveredServer(
                                name = serviceInfo.serviceName,
                                host = host,
                                port = serviceInfo.port,
                                fingerprint = fingerprint,
                                serverId = serverId,
                                version = version
                            )

                            discoveredServers[serviceInfo.serviceName] = server
                            _servers.value = discoveredServers.values.toList()

                            Timber.d(
                                "Server added name=%s host=%s port=%d id=%s fingerprint=%s",
                                server.name,
                                server.host,
                                server.port,
                                server.serverId,
                                server.fingerprint
                            )
                        }
                    }
                    nsdManager.resolveService(serviceInfo, resolveListener)
                }
            }

            override fun onServiceLost(serviceInfo: NsdServiceInfo) {
                Timber.d("Service lost: ${serviceInfo.serviceName}")
                discoveredServers.remove(serviceInfo.serviceName)
                _servers.value = discoveredServers.values.toList()
            }

            override fun onDiscoveryStopped(serviceType: String) {
                Timber.d("Discovery stopped: $serviceType")
                isDiscovering = false
            }

            override fun onStartDiscoveryFailed(serviceType: String, errorCode: Int) {
                Timber.e("Failed to start discovery: error $errorCode")
                isDiscovering = false
            }

            override fun onStopDiscoveryFailed(serviceType: String, errorCode: Int) {
                Timber.e("Failed to stop discovery: error $errorCode")
            }
        }

        try {
            nsdManager.discoverServices(
                SERVICE_TYPE,
                NsdManager.PROTOCOL_DNS_SD,
                discoveryListener
            )
        } catch (e: Exception) {
            Timber.e(e, "Failed to start NSD discovery")
            isDiscovering = false
            discoveryListener = null
            throw e
        }
    }

    override suspend fun stopDiscovery() {
        if (!isDiscovering) {
            Timber.w("Discovery not in progress")
            return
        }

        Timber.i("Stopping NSD discovery")

        discoveryListener?.let { listener ->
            try {
                nsdManager.stopServiceDiscovery(listener)
                discoveryListener = null
                discoveredServers.clear()
                _servers.value = emptyList()
            } catch (e: Exception) {
                Timber.e(e, "Failed to stop NSD discovery")
            }
        }
    }

    private fun extractFingerprint(serviceInfo: NsdServiceInfo): String? {
        return serviceInfo.attributes?.get("cert_fingerprint")?.let { bytes ->
            String(bytes, Charsets.UTF_8)
        }
    }

    private fun extractServerId(serviceInfo: NsdServiceInfo): String? {
        return serviceInfo.attributes?.get("server_id")?.let { bytes ->
            String(bytes, Charsets.UTF_8)
        }
    }

    private fun extractVersion(serviceInfo: NsdServiceInfo): String? {
        return serviceInfo.attributes?.get("version")?.let { bytes ->
            String(bytes, Charsets.UTF_8)
        }
    }

    companion object {
        private const val SERVICE_TYPE = "_handcontrol._tcp."
    }
}

package com.handcontrol.core.network

import io.grpc.ManagedChannel

interface GrpcChannelFactory {
    suspend fun createChannel(
        host: String,
        port: Int,
        expectedFingerprint: String? = null
    ): ManagedChannel
    suspend fun shutdownChannel(channel: ManagedChannel)
}

package com.handcontrol.core.network

import io.grpc.ManagedChannel

interface GrpcChannelFactory {
    suspend fun createChannel(host: String, port: Int): ManagedChannel
    suspend fun shutdownChannel(channel: ManagedChannel)
}

package com.handcontrol.di

import com.handcontrol.core.network.GrpcChannelFactory
import com.handcontrol.core.network.MtlsGrpcChannelFactory
import com.handcontrol.core.network.ServerHealthChecker
import com.handcontrol.core.network.ServerHealthCheckerImpl
import dagger.Binds
import dagger.Module
import dagger.hilt.InstallIn
import dagger.hilt.components.SingletonComponent
import javax.inject.Singleton

@Module
@InstallIn(SingletonComponent::class)
abstract class NetworkModule {

    @Binds
    @Singleton
    abstract fun bindGrpcChannelFactory(
        impl: MtlsGrpcChannelFactory
    ): GrpcChannelFactory

    @Binds
    @Singleton
    abstract fun bindServerHealthChecker(
        impl: ServerHealthCheckerImpl
    ): ServerHealthChecker
}

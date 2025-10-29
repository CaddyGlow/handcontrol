package com.handcontrol.di

import com.handcontrol.core.discovery.AndroidNsdDiscoveryManager
import com.handcontrol.core.discovery.NsdDiscoveryManager
import dagger.Binds
import dagger.Module
import dagger.hilt.InstallIn
import dagger.hilt.components.SingletonComponent
import javax.inject.Singleton

@Module
@InstallIn(SingletonComponent::class)
abstract class DiscoveryModule {

    @Binds
    @Singleton
    abstract fun bindNsdDiscoveryManager(
        impl: AndroidNsdDiscoveryManager
    ): NsdDiscoveryManager
}

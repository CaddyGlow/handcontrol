package com.handcontrol.di

import com.handcontrol.core.security.AndroidKeystoreCertificateManager
import com.handcontrol.core.security.ClientCertificateManager
import dagger.Binds
import dagger.Module
import dagger.hilt.InstallIn
import dagger.hilt.components.SingletonComponent
import javax.inject.Singleton

@Module
@InstallIn(SingletonComponent::class)
abstract class SecurityModule {

    @Binds
    @Singleton
    abstract fun bindClientCertificateManager(
        impl: AndroidKeystoreCertificateManager
    ): ClientCertificateManager
}

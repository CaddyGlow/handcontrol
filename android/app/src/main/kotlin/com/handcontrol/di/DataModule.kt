package com.handcontrol.di

import com.handcontrol.data.commands.CommandRepository
import com.handcontrol.data.commands.GrpcCommandRepository
import com.handcontrol.data.enrollment.EnrollmentRepository
import com.handcontrol.data.enrollment.GrpcEnrollmentRepository
import dagger.Binds
import dagger.Module
import dagger.hilt.InstallIn
import dagger.hilt.components.SingletonComponent
import javax.inject.Singleton

@Module
@InstallIn(SingletonComponent::class)
abstract class DataModule {

    @Binds
    @Singleton
    abstract fun bindCommandRepository(
        impl: GrpcCommandRepository
    ): CommandRepository

    @Binds
    @Singleton
    abstract fun bindEnrollmentRepository(
        impl: GrpcEnrollmentRepository
    ): EnrollmentRepository
}

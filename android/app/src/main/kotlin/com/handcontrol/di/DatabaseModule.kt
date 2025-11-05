package com.handcontrol.di

import android.content.Context
import androidx.room.Room
import com.handcontrol.data.database.EnrolledServerDao
import com.handcontrol.data.database.HandControlDatabase
import dagger.Module
import dagger.Provides
import dagger.hilt.InstallIn
import dagger.hilt.android.qualifiers.ApplicationContext
import dagger.hilt.components.SingletonComponent
import javax.inject.Singleton

@Module
@InstallIn(SingletonComponent::class)
object DatabaseModule {

    @Provides
    @Singleton
    fun provideHandControlDatabase(
        @ApplicationContext context: Context
    ): HandControlDatabase {
        return Room.databaseBuilder(
            context,
            HandControlDatabase::class.java,
            "handcontrol_database"
        )
            .addMigrations(
                HandControlDatabase.MIGRATION_1_2,
                HandControlDatabase.MIGRATION_2_3,
                HandControlDatabase.MIGRATION_3_4
            )
            .build()
    }

    @Provides
    @Singleton
    fun provideEnrolledServerDao(
        database: HandControlDatabase
    ): EnrolledServerDao {
        return database.enrolledServerDao()
    }
}

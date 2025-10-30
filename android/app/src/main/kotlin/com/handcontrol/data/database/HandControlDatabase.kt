package com.handcontrol.data.database

import androidx.room.Database
import androidx.room.RoomDatabase

@Database(
    entities = [EnrolledServerEntity::class],
    version = 1,
    exportSchema = false
)
abstract class HandControlDatabase : RoomDatabase() {
    abstract fun enrolledServerDao(): EnrolledServerDao
}

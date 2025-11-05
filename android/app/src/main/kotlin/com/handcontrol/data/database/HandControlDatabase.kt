package com.handcontrol.data.database

import androidx.room.Database
import androidx.room.RoomDatabase
import androidx.room.TypeConverters
import androidx.room.migration.Migration
import androidx.sqlite.db.SupportSQLiteDatabase

@Database(
    entities = [EnrolledServerEntity::class],
    version = 4,
    exportSchema = false
)
@TypeConverters(Converters::class)
abstract class HandControlDatabase : RoomDatabase() {
    abstract fun enrolledServerDao(): EnrolledServerDao

    companion object {
        // Migration from version 1 (single host) to version 2 (multi-IP + relay)
        val MIGRATION_1_2 = object : Migration(1, 2) {
            override fun migrate(database: SupportSQLiteDatabase) {
                // Add new columns
                database.execSQL("ALTER TABLE enrolled_servers ADD COLUMN ips TEXT NOT NULL DEFAULT ''")
                database.execSQL("ALTER TABLE enrolled_servers ADD COLUMN relayEnabled INTEGER NOT NULL DEFAULT 0")
                database.execSQL("ALTER TABLE enrolled_servers ADD COLUMN relayUrl TEXT")
                database.execSQL("ALTER TABLE enrolled_servers ADD COLUMN relayToken TEXT")
                database.execSQL("ALTER TABLE enrolled_servers ADD COLUMN lastConnectionMode TEXT NOT NULL DEFAULT 'UNKNOWN'")

                // Migrate existing serverHost values to ips (backward compatibility)
                database.execSQL("UPDATE enrolled_servers SET ips = serverHost WHERE serverHost IS NOT NULL AND serverHost != ''")
            }
        }

        val MIGRATION_2_3 = object : Migration(2, 3) {
            override fun migrate(database: SupportSQLiteDatabase) {
                database.execSQL("ALTER TABLE enrolled_servers ADD COLUMN connectionPreference TEXT NOT NULL DEFAULT 'AUTO'")
            }
        }

        val MIGRATION_3_4 = object : Migration(3, 4) {
            override fun migrate(database: SupportSQLiteDatabase) {
                database.execSQL("ALTER TABLE enrolled_servers ADD COLUMN remoteLayoutSpec TEXT")
            }
        }
    }
}

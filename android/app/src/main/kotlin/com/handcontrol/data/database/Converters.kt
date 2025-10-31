package com.handcontrol.data.database

import androidx.room.TypeConverter

class Converters {
    @TypeConverter
    fun fromStringList(value: List<String>?): String {
        return value?.joinToString(",") ?: ""
    }

    @TypeConverter
    fun toStringList(value: String): List<String> {
        return if (value.isEmpty()) emptyList() else value.split(",")
    }

    @TypeConverter
    fun fromConnectionMode(value: ConnectionMode): String {
        return value.name
    }

    @TypeConverter
    fun toConnectionMode(value: String): ConnectionMode {
        return try {
            ConnectionMode.valueOf(value)
        } catch (e: IllegalArgumentException) {
            ConnectionMode.UNKNOWN
        }
    }

    @TypeConverter
    fun fromConnectionPreference(value: ConnectionPreference): String {
        return value.name
    }

    @TypeConverter
    fun toConnectionPreference(value: String): ConnectionPreference {
        return try {
            ConnectionPreference.valueOf(value)
        } catch (e: IllegalArgumentException) {
            ConnectionPreference.AUTO
        }
    }
}

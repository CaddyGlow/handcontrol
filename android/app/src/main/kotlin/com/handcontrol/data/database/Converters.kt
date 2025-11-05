package com.handcontrol.data.database

import androidx.room.TypeConverter
import com.handcontrol.feature.commands.remote.RemoteLayoutSpec
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json
import kotlinx.serialization.decodeFromString

class Converters {
    private val json = Json {
        ignoreUnknownKeys = true
        encodeDefaults = false
        classDiscriminator = "kind"
    }

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

    @TypeConverter
    fun fromRemoteLayoutSpec(spec: RemoteLayoutSpec?): String? {
        return spec?.let { json.encodeToString(it) }
    }

    @TypeConverter
    fun toRemoteLayoutSpec(value: String?): RemoteLayoutSpec? {
        if (value.isNullOrBlank()) return null
        return try {
            json.decodeFromString<RemoteLayoutSpec>(value)
        } catch (_: Exception) {
            null
        }
    }
}

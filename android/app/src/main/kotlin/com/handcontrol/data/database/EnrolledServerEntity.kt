package com.handcontrol.data.database

import androidx.room.ColumnInfo
import androidx.room.Entity
import androidx.room.Index
import androidx.room.PrimaryKey

@Entity(
    tableName = "enrolled_servers",
    indices = [
        Index(value = ["last_connected"], name = "idx_last_connected"),
        Index(value = ["serverHost", "serverPort"], name = "idx_host_port")
    ]
)
data class EnrolledServerEntity(
    @PrimaryKey
    val serverId: String,

    val serverHost: String,
    val serverPort: Int,
    val clientId: String,
    val serverName: String,
    val certFingerprint: String,

    @ColumnInfo(name = "enrolled_at")
    val enrolledAt: Long,

    @ColumnInfo(name = "last_connected")
    val lastConnected: Long?
)

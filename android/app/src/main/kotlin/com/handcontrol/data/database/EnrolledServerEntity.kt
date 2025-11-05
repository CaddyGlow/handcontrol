package com.handcontrol.data.database

import androidx.room.ColumnInfo
import androidx.room.Entity
import androidx.room.Index
import androidx.room.PrimaryKey
import com.handcontrol.feature.commands.remote.RemoteLayoutSpec

enum class ConnectionMode {
    DIRECT,      // Connected via direct IP
    RELAY,       // Connected via relay server
    UNKNOWN      // Unknown/not yet connected
}

enum class ConnectionPreference {
    AUTO,          // Try direct first, fall back to relay
    DIRECT_ONLY,   // Use direct connections only
    RELAY_ONLY     // Force relay, skip direct attempts
}

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

    // Multi-IP support (new field)
    val ips: List<String>,

    val serverPort: Int,
    val clientId: String,
    val serverName: String,
    val certFingerprint: String,

    @ColumnInfo(name = "enrolled_at")
    val enrolledAt: Long,

    @ColumnInfo(name = "last_connected")
    val lastConnected: Long?,

    // Relay support fields (optional, for future use)
    val relayEnabled: Boolean = false,
    val relayUrl: String? = null,
    val relayToken: String? = null,
    val connectionPreference: ConnectionPreference = ConnectionPreference.AUTO,
    val lastConnectionMode: ConnectionMode = ConnectionMode.UNKNOWN,
    val remoteLayoutSpec: RemoteLayoutSpec? = null,

    // Deprecated but kept for migration compatibility
    @Deprecated("Use ips instead")
    val serverHost: String? = null
)

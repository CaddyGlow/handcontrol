package com.handcontrol.data.database

import androidx.room.Dao
import androidx.room.Delete
import androidx.room.Insert
import androidx.room.OnConflictStrategy
import androidx.room.Query
import androidx.room.Transaction
import kotlinx.coroutines.flow.Flow

@Dao
interface EnrolledServerDao {
    @Query("SELECT * FROM enrolled_servers ORDER BY (last_connected IS NULL), last_connected DESC")
    fun getAllServers(): Flow<List<EnrolledServerEntity>>

    @Query("SELECT * FROM enrolled_servers WHERE serverId = :serverId")
    suspend fun getServerById(serverId: String): EnrolledServerEntity?

    @Query("SELECT * FROM enrolled_servers WHERE serverId = :serverId")
    fun observeServerById(serverId: String): Flow<EnrolledServerEntity?>

    @Query("SELECT * FROM enrolled_servers ORDER BY (last_connected IS NULL), last_connected DESC LIMIT 1")
    suspend fun getLastConnectedServer(): EnrolledServerEntity?

    @Query("SELECT * FROM enrolled_servers ORDER BY (last_connected IS NULL), last_connected DESC LIMIT 1")
    fun observeLastConnectedServer(): Flow<EnrolledServerEntity?>

    @Insert(onConflict = OnConflictStrategy.REPLACE)
    suspend fun insertServer(server: EnrolledServerEntity)

    @Query("UPDATE enrolled_servers SET last_connected = :timestamp WHERE serverId = :serverId")
    suspend fun updateLastConnected(serverId: String, timestamp: Long)

    @Query("UPDATE enrolled_servers SET last_connected = :timestamp, lastConnectionMode = :mode WHERE serverId = :serverId")
    suspend fun updateConnectionMode(serverId: String, timestamp: Long, mode: ConnectionMode)

    @Query("UPDATE enrolled_servers SET connectionPreference = :preference WHERE serverId = :serverId")
    suspend fun updateConnectionPreference(serverId: String, preference: ConnectionPreference)

    @Query("UPDATE enrolled_servers SET certFingerprint = :fingerprint WHERE serverId = :serverId")
    suspend fun updateCertFingerprint(serverId: String, fingerprint: String)

    @Delete
    suspend fun deleteServer(server: EnrolledServerEntity)

    @Query("DELETE FROM enrolled_servers WHERE serverId = :serverId")
    suspend fun deleteServerById(serverId: String)

    @Query("DELETE FROM enrolled_servers")
    suspend fun deleteAllServers()

    @Query("SELECT certFingerprint FROM enrolled_servers WHERE serverId = :serverId")
    suspend fun getServerFingerprint(serverId: String): String?

    @Query("SELECT * FROM enrolled_servers WHERE serverHost = :host AND serverPort = :port")
    suspend fun getServerByHostAndPort(host: String, port: Int): EnrolledServerEntity?

    @Query(
        """
        UPDATE enrolled_servers 
        SET ips = :ips, serverHost = :primaryHost 
        WHERE serverId = :serverId
        """
    )
    suspend fun updateServerIps(serverId: String, ips: List<String>, primaryHost: String?)

    @Transaction
    suspend fun mergeServerIpsTransactional(serverId: String, discoveredIps: List<String>) {
        if (discoveredIps.isEmpty()) {
            return
        }

        val server = getServerById(serverId) ?: return

        val sanitized = (server.ips + discoveredIps)
            .map { it.trim() }
            .filter { it.isNotEmpty() }
            .filterNot { it.equals("unknown", ignoreCase = true) }
            .distinct()

        if (sanitized.isEmpty() || sanitized == server.ips) {
            return
        }

        val primaryHost = sanitized.firstOrNull()
        updateServerIps(serverId, sanitized, primaryHost)
    }
}

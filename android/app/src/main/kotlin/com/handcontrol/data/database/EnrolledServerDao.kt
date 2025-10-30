package com.handcontrol.data.database

import androidx.room.Dao
import androidx.room.Delete
import androidx.room.Insert
import androidx.room.OnConflictStrategy
import androidx.room.Query
import kotlinx.coroutines.flow.Flow

@Dao
interface EnrolledServerDao {
    @Query("SELECT * FROM enrolled_servers ORDER BY (last_connected IS NULL), last_connected DESC")
    fun getAllServers(): Flow<List<EnrolledServerEntity>>

    @Query("SELECT * FROM enrolled_servers WHERE serverId = :serverId")
    suspend fun getServerById(serverId: String): EnrolledServerEntity?

    @Query("SELECT * FROM enrolled_servers ORDER BY (last_connected IS NULL), last_connected DESC LIMIT 1")
    suspend fun getLastConnectedServer(): EnrolledServerEntity?

    @Query("SELECT * FROM enrolled_servers ORDER BY (last_connected IS NULL), last_connected DESC LIMIT 1")
    fun observeLastConnectedServer(): Flow<EnrolledServerEntity?>

    @Insert(onConflict = OnConflictStrategy.REPLACE)
    suspend fun insertServer(server: EnrolledServerEntity)

    @Query("UPDATE enrolled_servers SET last_connected = :timestamp WHERE serverId = :serverId")
    suspend fun updateLastConnected(serverId: String, timestamp: Long)

    @Delete
    suspend fun deleteServer(server: EnrolledServerEntity)

    @Query("DELETE FROM enrolled_servers WHERE serverId = :serverId")
    suspend fun deleteServerById(serverId: String)

    @Query("SELECT certFingerprint FROM enrolled_servers WHERE serverId = :serverId")
    suspend fun getServerFingerprint(serverId: String): String?

    @Query("SELECT * FROM enrolled_servers WHERE serverHost = :host AND serverPort = :port")
    suspend fun getServerByHostAndPort(host: String, port: Int): EnrolledServerEntity?
}

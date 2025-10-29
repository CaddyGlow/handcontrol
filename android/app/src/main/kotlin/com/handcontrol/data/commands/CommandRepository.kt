package com.handcontrol.data.commands

import kotlinx.coroutines.flow.Flow

interface CommandRepository {
    /**
     * Get server information
     */
    suspend fun getServerInfo(host: String, port: Int): Result<ServerInfo>

    /**
     * List all available commands from the server
     */
    suspend fun listCommands(host: String, port: Int): Result<List<Command>>

    /**
     * Execute a command with parameters and stream the output
     * Returns a Flow that emits stdout, stderr, and final exit code
     */
    suspend fun executeCommand(
        host: String,
        port: Int,
        commandId: String,
        parameters: Map<String, String>
    ): Flow<CommandExecutionResult>
}

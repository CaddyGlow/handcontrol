package com.handcontrol.data.commands

import kotlinx.coroutines.flow.Flow

interface CommandRepository {
    /**
     * Get server information
     */
    suspend fun getServerInfo(serverId: String): Result<ServerInfo>

    /**
     * List all available commands from the server
     */
    suspend fun listCommands(serverId: String): Result<List<Command>>

    /**
     * Execute a command with parameters and stream the output
     * Returns a Flow that emits stdout, stderr, and final exit code
     */
    suspend fun executeCommand(
        serverId: String,
        commandId: String,
        parameters: Map<String, String>
    ): Flow<CommandExecutionResult>

    /**
     * Open a realtime shell session for interactive capabilities.
     */
    suspend fun openShellSession(
        serverId: String,
        commandId: String,
        parameters: Map<String, String>,
        terminalSize: TerminalSize? = null
    ): Result<ShellSession>
}

package com.handcontrol.feature.commands

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.handcontrol.data.commands.Command
import com.handcontrol.data.commands.CommandExecutionResult
import com.handcontrol.data.commands.CommandRepository
import com.handcontrol.data.commands.ServerInfo
import com.handcontrol.data.database.EnrolledServerRepository
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.catch
import kotlinx.coroutines.launch
import timber.log.Timber
import javax.inject.Inject

sealed interface CommandListUiState {
    data object Loading : CommandListUiState
    data class Success(
        val serverInfo: ServerInfo,
        val commands: List<Command>,
        val filteredCommands: List<Command>,
        val searchQuery: String = ""
    ) : CommandListUiState
    data class Error(val message: String) : CommandListUiState
}

sealed interface CommandExecutionState {
    data object Idle : CommandExecutionState
    data class Executing(
        val commandId: String,
        val commandName: String,
        val output: List<CommandOutputLine>
    ) : CommandExecutionState
    data class Completed(
        val commandId: String,
        val commandName: String,
        val exitCode: Int,
        val output: List<CommandOutputLine>
    ) : CommandExecutionState
    data class Failed(val message: String) : CommandExecutionState
}

data class CommandOutputLine(
    val text: String,
    val isError: Boolean,
    val timestamp: Long = System.currentTimeMillis()
)

@HiltViewModel
class CommandListViewModel @Inject constructor(
    private val commandRepository: CommandRepository,
    private val enrolledServerRepository: EnrolledServerRepository
) : ViewModel() {

    private val _uiState = MutableStateFlow<CommandListUiState>(CommandListUiState.Loading)
    val uiState: StateFlow<CommandListUiState> = _uiState.asStateFlow()

    private val _executionState = MutableStateFlow<CommandExecutionState>(CommandExecutionState.Idle)
    val executionState: StateFlow<CommandExecutionState> = _executionState.asStateFlow()

    private val _toastMessage = MutableSharedFlow<String>()
    val toastMessage: SharedFlow<String> = _toastMessage.asSharedFlow()

    private var currentHost: String? = null
    private var currentPort: Int? = null

    fun loadCommands(host: String, port: Int) {
        currentHost = host
        currentPort = port

        viewModelScope.launch {
            _uiState.value = CommandListUiState.Loading

            try {
                // Load server info
                val serverInfoResult = commandRepository.getServerInfo(host, port)
                if (serverInfoResult.isFailure) {
                    _uiState.value = CommandListUiState.Error(
                        serverInfoResult.exceptionOrNull()?.message ?: "Failed to get server info"
                    )
                    return@launch
                }

                val serverInfo = serverInfoResult.getOrThrow()
                Timber.i("Connected to server: ${serverInfo.hostname} v${serverInfo.version}")

                // Load commands
                val commandsResult = commandRepository.listCommands(host, port)
                if (commandsResult.isFailure) {
                    _uiState.value = CommandListUiState.Error(
                        commandsResult.exceptionOrNull()?.message ?: "Failed to load commands"
                    )
                    return@launch
                }

                val commands = commandsResult.getOrThrow()
                Timber.i("Loaded ${commands.size} commands")

                _uiState.value = CommandListUiState.Success(
                    serverInfo = serverInfo,
                    commands = commands,
                    filteredCommands = commands
                )

                // Update last connected timestamp
                enrolledServerRepository.updateLastConnected(serverInfo.serverId)
            } catch (e: Exception) {
                Timber.e(e, "Error loading commands")
                _uiState.value = CommandListUiState.Error(e.message ?: "Unknown error")
            }
        }
    }

    fun searchCommands(query: String) {
        val currentState = _uiState.value
        if (currentState !is CommandListUiState.Success) return

        val filtered = if (query.isBlank()) {
            currentState.commands
        } else {
            currentState.commands.filter { command ->
                command.name.contains(query, ignoreCase = true) ||
                command.description.contains(query, ignoreCase = true) ||
                command.tags.any { it.contains(query, ignoreCase = true) }
            }
        }

        _uiState.value = currentState.copy(
            filteredCommands = filtered,
            searchQuery = query
        )
    }

    fun executeCommand(commandId: String, commandName: String, parameters: Map<String, String>) {
        val host = currentHost ?: return
        val port = currentPort ?: return

        viewModelScope.launch {
            try {
                _executionState.value = CommandExecutionState.Executing(
                    commandId = commandId,
                    commandName = commandName,
                    output = emptyList()
                )

                Timber.i("Executing command: $commandId")

                commandRepository.executeCommand(host, port, commandId, parameters)
                    .collect { result ->
                        val currentState = _executionState.value
                        if (currentState !is CommandExecutionState.Executing) return@collect

                        when (result) {
                            is CommandExecutionResult.Output -> {
                                val newOutput = currentState.output + CommandOutputLine(
                                    text = result.text,
                                    isError = result.isError
                                )
                                _executionState.value = currentState.copy(output = newOutput)
                            }
                            is CommandExecutionResult.ExitCode -> {
                                Timber.i("Command completed with exit code: ${result.code}")
                                _executionState.value = CommandExecutionState.Completed(
                                    commandId = commandId,
                                    commandName = commandName,
                                    exitCode = result.code,
                                    output = currentState.output
                                )
                            }
                            is CommandExecutionResult.Error -> {
                                Timber.e("Command failed: ${result.message}")
                                _executionState.value = CommandExecutionState.Failed(result.message)
                            }
                        }
                    }
            } catch (e: Exception) {
                Timber.e(e, "Command execution exception")
                _executionState.value = CommandExecutionState.Failed(e.message ?: "Unknown error")
            }
        }
    }

    fun clearExecution() {
        _executionState.value = CommandExecutionState.Idle
    }

    fun retryLoad() {
        val host = currentHost ?: return
        val port = currentPort ?: return
        loadCommands(host, port)
    }

    /**
     * Execute command with mode-specific behavior (show/hide output)
     */
    fun executeCommandWithMode(
        commandId: String,
        commandName: String,
        parameters: Map<String, String>,
        showOutput: Boolean
    ) {
        val host = currentHost ?: return
        val port = currentPort ?: return

        viewModelScope.launch {
            try {
                if (showOutput) {
                    // Standard execution with output screen
                    _executionState.value = CommandExecutionState.Executing(
                        commandId = commandId,
                        commandName = commandName,
                        output = emptyList()
                    )
                }

                Timber.i("Executing command: $commandId (showOutput=$showOutput)")

                commandRepository.executeCommand(host, port, commandId, parameters)
                    .catch { e ->
                        Timber.e(e, "Command execution failed")
                        if (showOutput) {
                            _executionState.value = CommandExecutionState.Failed(
                                e.message ?: "Execution failed"
                            )
                        } else {
                            _toastMessage.emit("Error: ${e.message}")
                        }
                    }
                    .collect { result ->
                        when (result) {
                            is CommandExecutionResult.Output -> {
                                if (showOutput) {
                                    val currentState = _executionState.value
                                    if (currentState is CommandExecutionState.Executing) {
                                        val newOutput = currentState.output + CommandOutputLine(
                                            text = result.text,
                                            isError = result.isError
                                        )
                                        _executionState.value = currentState.copy(output = newOutput)
                                    }
                                }
                            }
                            is CommandExecutionResult.ExitCode -> {
                                Timber.i("Command completed with exit code: ${result.code}")
                                if (showOutput) {
                                    val currentState = _executionState.value
                                    if (currentState is CommandExecutionState.Executing) {
                                        _executionState.value = CommandExecutionState.Completed(
                                            commandId = commandId,
                                            commandName = commandName,
                                            exitCode = result.code,
                                            output = currentState.output
                                        )
                                    }
                                } else {
                                    // Show toast notification
                                    val message = if (result.code == 0) {
                                        "$commandName completed successfully"
                                    } else {
                                        "$commandName failed (exit code: ${result.code})"
                                    }
                                    _toastMessage.emit(message)
                                }
                            }
                            is CommandExecutionResult.Error -> {
                                Timber.e("Command failed: ${result.message}")
                                if (showOutput) {
                                    _executionState.value = CommandExecutionState.Failed(result.message)
                                } else {
                                    _toastMessage.emit("Error: ${result.message}")
                                }
                            }
                        }
                    }
            } catch (e: Exception) {
                Timber.e(e, "Command execution exception")
                if (showOutput) {
                    _executionState.value = CommandExecutionState.Failed(e.message ?: "Unknown error")
                } else {
                    _toastMessage.emit("Error: ${e.message}")
                }
            }
        }
    }

    /**
     * Fetch dynamic default value for a parameter by executing a command
     */
    suspend fun fetchDynamicDefault(
        command: String,
        pattern: String?,
        fallback: String
    ): String {
        val host = currentHost ?: return fallback
        val port = currentPort ?: return fallback

        return try {
            val outputBuilder = StringBuilder()

            // Execute the command and collect output
            commandRepository.executeCommand(host, port, "_dynamic_default", mapOf("_cmd" to command))
                .catch { e ->
                    Timber.w(e, "Failed to fetch dynamic default")
                    emit(CommandExecutionResult.Error(e.message ?: "Failed"))
                }
                .collect { result ->
                    when (result) {
                        is CommandExecutionResult.Output -> {
                            if (!result.isError) {
                                outputBuilder.append(result.text)
                            }
                        }
                        is CommandExecutionResult.ExitCode -> {
                            if (result.code != 0) {
                                Timber.w("Dynamic default command exited with code ${result.code}")
                            }
                        }
                        is CommandExecutionResult.Error -> {
                            Timber.w("Dynamic default error: ${result.message}")
                        }
                    }
                }

            val output = outputBuilder.toString().trim()

            // Apply pattern if provided
            if (pattern != null && pattern.isNotBlank()) {
                try {
                    val regex = Regex(pattern)
                    val match = regex.find(output)
                    match?.groupValues?.getOrNull(1) ?: fallback
                } catch (e: Exception) {
                    Timber.w(e, "Failed to apply pattern: $pattern")
                    fallback
                }
            } else {
                output.ifEmpty { fallback }
            }
        } catch (e: Exception) {
            Timber.w(e, "Failed to fetch dynamic default")
            fallback
        }
    }
}

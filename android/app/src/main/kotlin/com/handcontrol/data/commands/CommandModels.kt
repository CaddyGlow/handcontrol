package com.handcontrol.data.commands

import kotlinx.coroutines.flow.Flow

/**
 * Domain model for a capability exposed by the server.
 *
 * The Android app still calls these "commands" in the UI, but behind the scenes they
 * correspond to capability-oriented metadata returned by the new ListCapabilities RPC.
 */
data class Command(
    val id: String,
    val name: String,
    val description: String,
    val icon: String?,
    val tags: List<String>,
    val parameters: List<CommandParameter>,
    val requiresConfirmation: Boolean = false,
    val showOutput: Boolean = true,
    val privileged: Boolean = false,
    val kind: CommandKind = CommandKind.UNKNOWN,
    val sessionMode: CommandSessionMode = CommandSessionMode.UNSPECIFIED
)

/**
 * Domain model for command parameter
 */
data class CommandParameter(
    val name: String,
    val type: ParameterType,
    val description: String,
    val min: Int? = null,
    val max: Int? = null,
    val defaultValue: String? = null,
    val options: List<String> = emptyList(),
    val validation: String? = null,
    val labelOn: String? = null,
    val labelOff: String? = null,
    val defaultValueCommand: String? = null,
    val defaultValuePattern: String? = null
)

/**
 * Parameter types matching protobuf definition
 */
enum class ParameterType {
    SLIDER,
    TEXT,
    TOGGLE,
    DROPDOWN,
    UNSPECIFIED
}

/**
 * Capability kind as reported by the server.
 */
enum class CommandKind {
    SHELL_SCRIPT,
    SHELL_INTERACTIVE,
    FILE_TRANSFER,
    UNKNOWN
}

/**
 * Session mode required by a capability.
 */
enum class CommandSessionMode {
    ONE_SHOT,
    REALTIME,
    UPLOAD,
    DOWNLOAD,
    UNSPECIFIED
}

/**
 * Command execution result
 */
sealed interface CommandExecutionResult {
    data class Output(val text: String, val isError: Boolean = false) : CommandExecutionResult
    data class ExitCode(val code: Int) : CommandExecutionResult
    data class Error(val message: String) : CommandExecutionResult
}

/**
 * Server information
 */
data class ServerInfo(
    val serverId: String,
    val hostname: String,
    val version: String,
    val os: String
)

/**
 * Represents the current state of a parameter input in the UI
 */
data class ParameterInputState(
    val parameter: com.handcontrol.grpc.CapabilityParameter,
    val currentValue: String,
    val validationResult: ValidationResult,
    val isDirty: Boolean = false
)

/**
 * Result of parameter validation
 */
sealed class ValidationResult {
    object Valid : ValidationResult()
    data class Invalid(val message: String) : ValidationResult()
    object Pending : ValidationResult()
}

/**
 * Console dimensions for interactive shell sessions.
 */
data class TerminalSize(val cols: Int, val rows: Int)

/**
 * Events emitted during a realtime shell session.
 */
sealed class ShellSessionEvent {
    data class Ready(val capabilityId: String, val message: String?) : ShellSessionEvent()
    data class Output(
        val text: String,
        val isError: Boolean,
        val isBinary: Boolean,
        val timestampMs: Long?
    ) : ShellSessionEvent()
    data class Exit(val exitCode: Int, val timedOut: Boolean, val message: String?) : ShellSessionEvent()
    data class Error(val message: String, val code: Int? = null) : ShellSessionEvent()
    data class Heartbeat(val timestampMs: Long, val latencyHintMs: Long?) : ShellSessionEvent()
    data class Closed(val reason: String?) : ShellSessionEvent()
}

/**
 * Handle for controlling a realtime shell session.
 */
data class ShellSession(
    val events: Flow<ShellSessionEvent>,
    val writeInput: suspend (ByteArray) -> Unit,
    val resize: suspend (TerminalSize) -> Unit,
    val close: suspend (String?) -> Unit
)

package com.handcontrol.data.commands

/**
 * Domain model for a command available on the server
 */
data class Command(
    val id: String,
    val name: String,
    val description: String,
    val icon: String,
    val tags: List<String>,
    val parameters: List<CommandParameter>
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
    val labelOff: String? = null
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

package com.handcontrol.feature.commands.shell

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.handcontrol.data.commands.Command
import com.handcontrol.data.commands.CommandExecutionResult
import com.handcontrol.data.commands.CommandKind
import com.handcontrol.data.commands.CommandRepository
import com.handcontrol.data.commands.CommandSessionMode
import com.handcontrol.data.commands.ParameterInputState
import com.handcontrol.data.commands.ShellSession
import com.handcontrol.data.commands.ShellSessionEvent
import com.handcontrol.data.commands.TerminalSize
import com.handcontrol.data.commands.ValidationResult
import com.handcontrol.data.commands.defaultValueFor
import com.handcontrol.data.commands.toProtoParameter
import com.handcontrol.data.commands.validateParameterValue
import com.termux.terminal.RemoteTerminalSession
import com.termux.terminal.TerminalSession
import com.termux.terminal.TerminalSessionClient
import dagger.hilt.android.lifecycle.HiltViewModel
import javax.inject.Inject
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.catch
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import timber.log.Timber

private const val DEFAULT_TERMINAL_COLS = 120
private const val DEFAULT_TERMINAL_ROWS = 32

@HiltViewModel
class ShellSessionViewModel @Inject constructor(
    private val commandRepository: CommandRepository
) : ViewModel() {

    private val _uiState = MutableStateFlow(ShellSessionUiState())
    val uiState: StateFlow<ShellSessionUiState> = _uiState.asStateFlow()

    private val _toastMessages = MutableSharedFlow<String>()
    val toastMessages: SharedFlow<String> = _toastMessages.asSharedFlow()

    private var currentServerId: String? = null
    private var currentCommand: Command? = null
    private var parameterStates: Map<String, ParameterInputState> = emptyMap()
    private var dynamicLoadingStates: MutableMap<String, Boolean> = mutableMapOf()
    private var activeSession: ShellSession? = null
    private var remoteTerminalSession: RemoteTerminalSession? = null
    private var terminalBridge: ComposeTerminalBridgeHandle? = null

    fun load(serverId: String, commandId: String) {
        if (currentServerId == serverId && currentCommand?.id == commandId) return

        viewModelScope.launch {
            _uiState.value = ShellSessionUiState(isLoading = true)

            try {
                val commandsResult = commandRepository.listCommands(serverId)
                val command = commandsResult.getOrThrow().find { it.id == commandId }
                    ?: throw IllegalStateException("Capability not found on server")

                if (command.kind != CommandKind.SHELL_INTERACTIVE ||
                    command.sessionMode != CommandSessionMode.REALTIME
                ) {
                    throw IllegalStateException("${command.name} is not an interactive shell capability")
                }

                currentServerId = serverId
                currentCommand = command
                dynamicLoadingStates = mutableMapOf()

                val initialStates = command.parameters.associate { param ->
                    val proto = param.toProtoParameter()
                    val defaultValue = param.defaultValue ?: defaultValueFor(param.type)
                    val validation = validateParameterValue(proto, defaultValue)
                    param.name to ParameterInputState(
                        parameter = proto,
                        currentValue = defaultValue,
                        validationResult = validation,
                        isDirty = false
                    )
                }
                parameterStates = initialStates

                _uiState.value = ShellSessionUiState(
                    isLoading = false,
                    command = command,
                    parameterStates = initialStates,
                    dynamicLoading = dynamicLoadingStates.toMap(),
                    isFormValid = initialStates.values.all { it.validationResult is ValidationResult.Valid },
                    connectionState = ShellConnectionState.NotStarted,
                    errorMessage = null,
                    terminalSession = null,
                    statusMessage = null
                )

                // Kick off dynamic default fetches (non-blocking)
                command.parameters.forEach { param ->
                    if (param.defaultValueCommand != null) {
                        refreshDefault(param.name, quiet = true)
                    }
                }
            } catch (e: Exception) {
                Timber.e(e, "Failed to load shell capability")
                _uiState.value = ShellSessionUiState(
                    isLoading = false,
                    command = null,
                    parameterStates = emptyMap(),
                    dynamicLoading = emptyMap(),
                    isFormValid = false,
                    connectionState = ShellConnectionState.NotStarted,
                    errorMessage = e.message ?: "Failed to load capability",
                    terminalSession = null,
                    statusMessage = null
                )
            }
        }
    }

    fun updateParameter(name: String, value: String) {
        val current = parameterStates[name] ?: return
        val validation = validateParameterValue(current.parameter, value)
        val updated = current.copy(
            currentValue = value,
            validationResult = validation,
            isDirty = true
        )
        parameterStates = parameterStates + (name to updated)
        updateFormState()
    }

    fun refreshDefault(name: String, quiet: Boolean = false) {
        val serverId = currentServerId ?: return
        val command = currentCommand ?: return
        val parameter = command.parameters.find { it.name == name } ?: return
        val defaultCommand = parameter.defaultValueCommand ?: return
        val state = parameterStates[name] ?: return

        viewModelScope.launch {
            if (!quiet) {
                setDynamicLoading(name, true)
            }

            val fallback = state.currentValue
            val value = fetchDynamicDefault(
                serverId = serverId,
                commandId = command.id,
                shellCommand = defaultCommand,
                pattern = parameter.defaultValuePattern,
                fallback = fallback
            )

            parameterStates = parameterStates + (name to state.copy(
                currentValue = value,
                validationResult = validateParameterValue(state.parameter, value),
                isDirty = false
            ))

            setDynamicLoading(name, false)
            updateFormState()
        }
    }

    fun startSession() {
        val serverId = currentServerId
        val command = currentCommand

        if (serverId == null || command == null) {
            emitToast("Capability not loaded yet")
            return
        }

        if (activeSession != null) {
            emitToast("Session already active")
            return
        }

        val invalidParam = parameterStates.entries.firstOrNull {
            it.value.validationResult is ValidationResult.Invalid
        }
        if (invalidParam != null) {
            emitToast("Fix parameter '${invalidParam.key}' before starting the session")
            return
        }

        val parameters = parameterStates.mapValues { it.value.currentValue }

        viewModelScope.launch {
            // Reset output and mark as connecting
            _uiState.update {
                it.copy(
                    connectionState = ShellConnectionState.Connecting,
                    terminalSession = null,
                    statusMessage = null
                )
            }

            val result = commandRepository.openShellSession(
                serverId = serverId,
                commandId = command.id,
                parameters = parameters,
                terminalSize = TerminalSize(DEFAULT_TERMINAL_COLS, DEFAULT_TERMINAL_ROWS)
            )

            if (result.isFailure) {
                val message = result.exceptionOrNull()?.message ?: "Failed to open shell session"
                Timber.e(result.exceptionOrNull(), "Shell session failed to start")
                _uiState.update {
                    it.copy(connectionState = ShellConnectionState.Failed(message))
                }
                emitToast(message)
                return@launch
            }

            val shellSession = result.getOrThrow()
            activeSession = shellSession

            remoteTerminalSession?.finishIfRunning()

            val terminalSession = RemoteTerminalSession(
                scope = viewModelScope,
                shellSession = shellSession,
                transcriptRows = null,
                client = NoOpTerminalSessionClient,
                onEvent = ::handleTerminalEvent
            )

            remoteTerminalSession = terminalSession
            _uiState.update {
                it.copy(
                    connectionState = ShellConnectionState.Connecting,
                    terminalSession = terminalSession,
                    statusMessage = null
                )
            }
        }
    }

    fun sendText(text: String, appendNewline: Boolean = true) {
        val session = activeSession ?: run {
            emitToast("Session is not active")
            return
        }

        viewModelScope.launch {
            try {
                val payload = if (appendNewline) {
                    (text + "\n").toByteArray(Charsets.UTF_8)
                } else {
                    text.toByteArray(Charsets.UTF_8)
                }
                session.writeInput(payload)
            } catch (e: Exception) {
                Timber.e(e, "Failed to send shell input")
                emitToast(e.message ?: "Failed to send input")
            }
        }
    }

    fun sendCtrlC() {
        val session = activeSession ?: run {
            emitToast("Session is not active")
            return
        }

        viewModelScope.launch {
            try {
                session.writeInput(byteArrayOf(3)) // ASCII ETX
            } catch (e: Exception) {
                Timber.e(e, "Failed to send Ctrl+C")
                emitToast(e.message ?: "Failed to send Ctrl+C")
            }
        }
    }

    fun closeSession(reason: String? = null) {
        val session = remoteTerminalSession ?: return
        viewModelScope.launch {
            try {
                terminalBridge?.hideKeyboard()
                session.dispose(reason)
            } catch (e: Exception) {
                Timber.e(e, "Failed to close shell session")
                emitToast(e.message ?: "Failed to close session")
            } finally {
                remoteTerminalSession = null
                activeSession = null
                terminalBridge = null
                _uiState.update {
                    it.copy(
                        terminalSession = null,
                        connectionState = ShellConnectionState.Closed(reason),
                        statusMessage = reason
                    )
                }
            }
        }
    }

    fun registerTerminalBridge(handle: ComposeTerminalBridgeHandle) {
        terminalBridge = handle
    }

    fun toggleKeyboard() {
        terminalBridge?.toggleKeyboard()
    }

    override fun onCleared() {
        super.onCleared()
        remoteTerminalSession?.finishIfRunning()
        remoteTerminalSession = null
        activeSession = null
        terminalBridge = null
    }

    private fun updateFormState() {
        _uiState.update {
            it.copy(
                parameterStates = parameterStates,
                dynamicLoading = dynamicLoadingStates.toMap(),
                isFormValid = parameterStates.values.all { state ->
                    state.validationResult is ValidationResult.Valid
                }
            )
        }
    }

    private fun setDynamicLoading(name: String, loading: Boolean) {
        if (loading) {
            dynamicLoadingStates[name] = true
        } else {
            dynamicLoadingStates.remove(name)
        }
        _uiState.update {
            it.copy(dynamicLoading = dynamicLoadingStates.toMap())
        }
    }

    private suspend fun fetchDynamicDefault(
        serverId: String,
        commandId: String,
        shellCommand: String,
        pattern: String?,
        fallback: String
    ): String {
        return try {
            val outputBuilder = StringBuilder()
            commandRepository.executeCommand(
                serverId = serverId,
                commandId = "_dynamic_default",
                parameters = mapOf("_cmd" to shellCommand)
            )
                .catch { emit(CommandExecutionResult.Error(it.message ?: "Failed")) }
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

            val raw = outputBuilder.toString().trim()
            if (pattern.isNullOrBlank()) {
                raw.ifEmpty { fallback }
            } else {
                try {
                    val regex = Regex(pattern)
                    regex.find(raw)?.groupValues?.getOrNull(1)?.ifBlank { null } ?: fallback
                } catch (e: Exception) {
                    Timber.w(e, "Failed to apply dynamic default pattern for $commandId")
                    fallback
                }
            }
        } catch (e: Exception) {
            Timber.w(e, "Failed to fetch dynamic default for $commandId")
            fallback
        }
    }

    private fun emitToast(message: String) {
        viewModelScope.launch {
            _toastMessages.emit(message)
        }
    }

    private fun handleTerminalEvent(event: ShellSessionEvent) {
        when (event) {
            is ShellSessionEvent.Ready -> {
                _uiState.update {
                    it.copy(
                        connectionState = ShellConnectionState.Active(event.message),
                        statusMessage = event.message
                    )
                }
            }
            is ShellSessionEvent.Output -> {
                // Text rendering is handled by the terminal view directly.
            }
            is ShellSessionEvent.Exit -> {
                activeSession = null
                _uiState.update {
                    it.copy(
                        connectionState = ShellConnectionState.Completed(
                            exitCode = event.exitCode,
                            message = event.message
                        ),
                        statusMessage = event.message
                    )
                }
            }
            is ShellSessionEvent.Error -> {
                activeSession = null
                _uiState.update {
                    it.copy(
                        connectionState = ShellConnectionState.Failed(event.message),
                        statusMessage = event.message
                    )
                }
                emitToast(event.message)
            }
            is ShellSessionEvent.Heartbeat -> {
                // No-op; we may surface latency information later.
            }
            is ShellSessionEvent.Closed -> {
                activeSession = null
                val message = event.reason ?: "Session closed"
                _uiState.update {
                    it.copy(
                        connectionState = ShellConnectionState.Closed(event.reason),
                        statusMessage = message
                    )
                }
            }
        }
    }
}

sealed class ShellConnectionState {
    data object NotStarted : ShellConnectionState()
    data object Connecting : ShellConnectionState()
    data class Active(val readyMessage: String?) : ShellConnectionState()
    data class Completed(val exitCode: Int, val message: String?) : ShellConnectionState()
    data class Failed(val message: String) : ShellConnectionState()
    data class Closed(val reason: String?) : ShellConnectionState()
}

data class ShellSessionUiState(
    val isLoading: Boolean = true,
    val command: Command? = null,
    val parameterStates: Map<String, ParameterInputState> = emptyMap(),
    val dynamicLoading: Map<String, Boolean> = emptyMap(),
    val isFormValid: Boolean = false,
    val connectionState: ShellConnectionState = ShellConnectionState.NotStarted,
    val errorMessage: String? = null,
    val terminalSession: TerminalSession? = null,
    val statusMessage: String? = null
)

private object NoOpTerminalSessionClient : TerminalSessionClient {
    override fun onTextChanged(changedSession: TerminalSession) = Unit
    override fun onTitleChanged(changedSession: TerminalSession) = Unit
    override fun onSessionFinished(finishedSession: TerminalSession) = Unit
    override fun onCopyTextToClipboard(session: TerminalSession, text: String) = Unit
    override fun onPasteTextFromClipboard(session: TerminalSession) = Unit
    override fun onBell(session: TerminalSession) = Unit
    override fun onColorsChanged(session: TerminalSession) = Unit
    override fun onTerminalCursorStateChange(state: Boolean) = Unit
    override fun getTerminalCursorStyle(): Int? = null
    override fun logError(tag: String, message: String) = Timber.e("$tag: $message")
    override fun logWarn(tag: String, message: String) = Timber.w("$tag: $message")
    override fun logInfo(tag: String, message: String) = Timber.i("$tag: $message")
    override fun logDebug(tag: String, message: String) = Timber.d("$tag: $message")
    override fun logVerbose(tag: String, message: String) = Timber.v("$tag: $message")
    override fun logStackTraceWithMessage(tag: String, message: String, e: Exception) =
        Timber.e(e, "$tag: $message")
    override fun logStackTrace(tag: String, e: Exception) = Timber.e(e, tag)
}

interface ComposeTerminalBridgeHandle {
    fun toggleKeyboard()
    fun hideKeyboard()
}

package com.handcontrol.feature.commands

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.handcontrol.data.commands.Command
import com.handcontrol.data.commands.CommandExecutionResult
import com.handcontrol.data.commands.CommandRepository
import com.handcontrol.data.commands.ServerInfo
import com.handcontrol.data.commands.ParameterType
import com.handcontrol.data.database.EnrolledServerRepository
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.Job
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.catch
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import timber.log.Timber
import javax.inject.Inject
import com.handcontrol.feature.commands.remote.RemoteLayoutEntry
import com.handcontrol.feature.commands.remote.RemoteLayoutEntry.AdjustmentControl
import com.handcontrol.feature.commands.remote.RemoteLayoutEntry.TelemetryDisplay
import com.handcontrol.feature.commands.remote.RemoteLayoutSpec
import com.handcontrol.feature.commands.remote.RemotePanelUiModel
import com.handcontrol.feature.commands.remote.RemoteQuickAction
import com.handcontrol.feature.commands.remote.RemoteAdjustmentGroup
import com.handcontrol.feature.commands.remote.RemoteAdjustment
import com.handcontrol.feature.commands.remote.RemoteTelemetryCard
import kotlin.math.abs

sealed interface CommandListUiState {
    data object Loading : CommandListUiState
    data class Success(
        val serverInfo: ServerInfo,
        val commands: List<Command>,
        val filteredCommands: List<Command>,
        val searchQuery: String = "",
        val remotePanel: RemotePanelUiModel = RemotePanelUiModel.empty(),
        val remoteLayoutSpec: RemoteLayoutSpec? = null
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

    private var currentServerId: String? = null
    private var telemetrySources: Map<String, TelemetrySource> = emptyMap()
    private val telemetryJobs = mutableMapOf<String, Job>()
    private val telemetryLatestCards = mutableMapOf<String, RemoteTelemetryCard>()

    fun loadCommands(serverId: String) {
        currentServerId = serverId

        viewModelScope.launch {
            _uiState.value = CommandListUiState.Loading

            try {
                // Load server info
                val serverInfoResult = commandRepository.getServerInfo(serverId)
                if (serverInfoResult.isFailure) {
                    _uiState.value = CommandListUiState.Error(
                        serverInfoResult.exceptionOrNull()?.message ?: "Failed to get server info"
                    )
                    return@launch
                }

                val serverInfo = serverInfoResult.getOrThrow()
                Timber.i("Connected to server: ${serverInfo.hostname} v${serverInfo.version}")

                // Load commands
                val commandsResult = commandRepository.listCommands(serverId)
                if (commandsResult.isFailure) {
                    _uiState.value = CommandListUiState.Error(
                        commandsResult.exceptionOrNull()?.message ?: "Failed to load commands"
                    )
                    return@launch
                }

                val commands = commandsResult.getOrThrow()
                Timber.i("Loaded ${commands.size} commands")

                val layoutSpec = enrolledServerRepository.getRemoteLayoutSpec(serverId)

                _uiState.value = CommandListUiState.Success(
                    serverInfo = serverInfo,
                    commands = commands,
                    filteredCommands = commands,
                    remotePanel = buildRemotePanelModel(commands, layoutSpec),
                    remoteLayoutSpec = layoutSpec
                )

                // Note: Connection mode is already updated by repository
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
        val serverId = currentServerId ?: return

        viewModelScope.launch {
            try {
                _executionState.value = CommandExecutionState.Executing(
                    commandId = commandId,
                    commandName = commandName,
                    output = emptyList()
                )

                Timber.i("Executing command: $commandId")

                commandRepository.executeCommand(serverId, commandId, parameters)
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
        val serverId = currentServerId ?: return
        loadCommands(serverId)
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
        val serverId = currentServerId ?: return

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

                commandRepository.executeCommand(serverId, commandId, parameters)
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
        val serverId = currentServerId ?: return fallback

        return try {
            val outputBuilder = StringBuilder()

            // Execute the command and collect output
            commandRepository.executeCommand(serverId, "_dynamic_default", mapOf("_cmd" to command))
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

    fun saveRemoteLayoutSpec(spec: RemoteLayoutSpec) {
        val serverId = currentServerId ?: return

        viewModelScope.launch {
            enrolledServerRepository.saveRemoteLayoutSpec(serverId, spec)

            val currentState = _uiState.value
            if (currentState is CommandListUiState.Success) {
                val updatedPanel = buildRemotePanelModel(currentState.commands, spec)
                _uiState.value = currentState.copy(
                    remotePanel = updatedPanel,
                    remoteLayoutSpec = spec
                )
            }
        }
    }

    fun clearRemoteLayoutSpec() {
        val serverId = currentServerId ?: return

        viewModelScope.launch {
            enrolledServerRepository.saveRemoteLayoutSpec(serverId, null)

            val currentState = _uiState.value
            if (currentState is CommandListUiState.Success) {
                val fallbackPanel = buildRemotePanelModel(currentState.commands, null)
                _uiState.value = currentState.copy(
                    remotePanel = fallbackPanel,
                    remoteLayoutSpec = null
                )
            }
        }
    }

    fun triggerQuickCommand(
        command: Command,
        parameters: Map<String, String> = emptyMap(),
        showOutputOverride: Boolean? = null
    ) {
        val showOutput = showOutputOverride ?: command.showOutput

        // Visual feedback is handled by UI component
        // Toast notification is shown on completion in executeCommandWithMode
        executeCommandWithMode(
            commandId = command.id,
            commandName = command.name,
            parameters = parameters,
            showOutput = showOutput
        )
    }

    override fun onCleared() {
        super.onCleared()
        telemetryJobs.values.forEach { it.cancel() }
        telemetryJobs.clear()
    }

    private fun buildRemotePanelModel(
        commands: List<Command>,
        layoutSpec: RemoteLayoutSpec?
    ): RemotePanelUiModel {
        if (layoutSpec == null || layoutSpec.sections.isEmpty()) {
            return buildDefaultRemotePanel(commands)
        }

        val commandMap = commands.associateBy { it.id }
        val quickActions = mutableListOf<RemoteQuickAction>()
        val adjustmentGroups = mutableListOf<RemoteAdjustmentGroup>()
        val telemetryCards = mutableListOf<RemoteTelemetryCard>()
        val newTelemetrySources = mutableMapOf<String, TelemetrySource>()

        layoutSpec.sections.forEach { section ->
            when (section.type) {
                RemoteLayoutSpec.SectionType.QUICK_ACTIONS -> {
                    val sectionActions = section.entries
                        .filterIsInstance<RemoteLayoutEntry.QuickActionEntry>()
                        .mapNotNull { entry ->
                            val command = commandMap[entry.commandId] ?: return@mapNotNull null

                            RemoteQuickAction(
                                id = command.id,
                                title = entry.labelOverride ?: command.name,
                                subtitle = command.description.takeIf { it.isNotBlank() },
                                iconKey = entry.iconOverride ?: command.icon.takeIf { it.isNotBlank() },
                                isEnabled = true,
                                isBusy = false,
                                requiresConfirmation = entry.requiresConfirmationOverride
                                    ?: command.requiresConfirmation,
                                showOutput = entry.showOutputOverride ?: command.showOutput
                            )
                        }
                    quickActions.addAll(sectionActions)
                }

                RemoteLayoutSpec.SectionType.ADJUSTMENTS -> {
                    val controls = section.entries
                        .filterIsInstance<RemoteLayoutEntry.AdjustmentEntry>()
                        .mapNotNull { entry ->
                            val command = commandMap[entry.commandId] ?: return@mapNotNull null
                            mapAdjustmentEntry(entry, command)
                        }

                    if (controls.isNotEmpty()) {
                        adjustmentGroups.add(
                            RemoteAdjustmentGroup(
                                id = section.id,
                                title = section.title,
                                controls = controls
                            )
                        )
                    }
                }

                RemoteLayoutSpec.SectionType.TELEMETRY -> {
                    section.entries
                        .filterIsInstance<RemoteLayoutEntry.TelemetryEntry>()
                        .forEach { entry ->
                            val command = commandMap[entry.sourceCommandId] ?: return@forEach
                            val baseCard = mapTelemetryEntry(entry, command)
                            val previous = telemetryLatestCards[baseCard.id]
                            telemetryCards += previous ?: baseCard
                            newTelemetrySources[baseCard.id] = TelemetrySource(command, entry)
                        }
                }
            }
        }

        refreshTelemetrySources(newTelemetrySources)

        if (quickActions.isEmpty() && adjustmentGroups.isEmpty() && telemetryCards.isEmpty()) {
            return buildDefaultRemotePanel(commands)
        }

        return RemotePanelUiModel(
            quickActions = quickActions,
            adjustments = adjustmentGroups,
            telemetry = telemetryCards
        )
    }

    private fun buildDefaultRemotePanel(commands: List<Command>): RemotePanelUiModel {
        refreshTelemetrySources(emptyMap())

        val quickActions = commands
            .filter { it.parameters.isEmpty() }
            .take(6)
            .map { command ->
                RemoteQuickAction(
                    id = command.id,
                    title = command.name,
                    subtitle = command.description.takeIf { it.isNotBlank() },
                    iconKey = command.icon.takeIf { it.isNotBlank() },
                    isEnabled = true,
                    isBusy = false,
                    requiresConfirmation = command.requiresConfirmation,
                    showOutput = command.showOutput
                )
            }

        val adjustmentGroups = commands
            .filter { it.parameters.isNotEmpty() }
            .take(3)
            .mapNotNull { command ->
                val controls = command.parameters.mapNotNull { parameter ->
                    when (parameter.type) {
                        ParameterType.SLIDER -> {
                            val min = parameter.min?.toFloat() ?: 0f
                            val max = parameter.max?.toFloat() ?: 100f
                            val safeRange = if (min < max) min..max else 0f..100f
                            RemoteAdjustment.Slider(
                                id = "${command.id}:${parameter.name}",
                                label = parameter.name,
                                isEnabled = true,
                                value = parameter.defaultValue?.toFloatOrNull()
                                    ?.coerceIn(safeRange.start, safeRange.endInclusive)
                                    ?: safeRange.start,
                                range = safeRange,
                                step = null,
                                helperText = parameter.description.takeIf { it.isNotBlank() }
                            )
                        }

                        ParameterType.TOGGLE -> RemoteAdjustment.Toggle(
                            id = "${command.id}:${parameter.name}",
                            label = parameter.name,
                            isEnabled = true,
                            isChecked = parseBoolean(parameter.defaultValue) ?: false,
                            helperText = parameter.description.takeIf { it.isNotBlank() }
                        )

                        ParameterType.TEXT -> null
                        ParameterType.DROPDOWN -> null
                        ParameterType.UNSPECIFIED -> null
                    }
                }

                if (controls.isEmpty()) {
                    null
                } else {
                    RemoteAdjustmentGroup(
                        id = command.id,
                        title = command.name.takeIf { it.isNotBlank() },
                        controls = controls
                    )
                }
            }

        return RemotePanelUiModel(
            quickActions = quickActions,
            adjustments = adjustmentGroups,
            telemetry = emptyList()
        )
    }

    private fun refreshTelemetrySources(newSources: Map<String, TelemetrySource>) {
        val removedIds = telemetrySources.keys - newSources.keys
        removedIds.forEach { id ->
            telemetryJobs.remove(id)?.cancel()
            telemetryLatestCards.remove(id)
        }

        val serverId = currentServerId

        newSources.forEach { (id, source) ->
            val existing = telemetrySources[id]
            if (existing != source) {
                telemetryJobs.remove(id)?.cancel()
                telemetryLatestCards.remove(id)
                if (serverId != null) {
                    telemetryJobs[id] = viewModelScope.launch {
                        collectTelemetry(serverId, id, source)
                    }
                }
            }
        }

        telemetrySources = newSources
    }

    private fun mapAdjustmentEntry(
        entry: RemoteLayoutEntry.AdjustmentEntry,
        command: Command
    ): RemoteAdjustment? {
        val parameter = command.parameters.find { it.name == entry.parameterName } ?: return null
        val helperText = entry.helperText ?: parameter.description.takeIf { it.isNotBlank() }

        val resolvedControl = when (entry.control) {
            AdjustmentControl.AUTO -> when (parameter.type) {
                ParameterType.SLIDER -> AdjustmentControl.SLIDER
                ParameterType.TOGGLE -> AdjustmentControl.TOGGLE
                else -> AdjustmentControl.COUNTER
            }

            else -> entry.control
        }

        return when (resolvedControl) {
            AdjustmentControl.SLIDER -> {
                val min = entry.slider?.min ?: parameter.min?.toFloat() ?: 0f
                val max = entry.slider?.max ?: parameter.max?.toFloat() ?: (if (min < 100f) 100f else min + 1f)
                val (start, end) = if (min < max) min to max else 0f to 100f
                val value = parameter.defaultValue?.toFloatOrNull()
                    ?.takeIf { it in start..end }
                    ?: start

                RemoteAdjustment.Slider(
                    id = "${command.id}:${parameter.name}",
                    label = entry.labelOverride ?: parameter.name,
                    isEnabled = true,
                    value = value,
                    range = start..end,
                    step = entry.slider?.step,
                    helperText = helperText
                )
            }

            AdjustmentControl.TOGGLE -> {
                val isChecked = parseBoolean(parameter.defaultValue) ?: false
                val derivedHelper = helperText ?: run {
                    val toggleOverrides = entry.toggle
                    when {
                        toggleOverrides == null -> null
                        isChecked -> toggleOverrides.checkedLabel
                        else -> toggleOverrides.uncheckedLabel
                    }
                }

                RemoteAdjustment.Toggle(
                    id = "${command.id}:${parameter.name}",
                    label = entry.labelOverride ?: parameter.name,
                    isEnabled = true,
                    isChecked = isChecked,
                    helperText = derivedHelper
                )
            }

            AdjustmentControl.COUNTER -> {
                val counterOverrides = entry.counter
                val min = counterOverrides?.min ?: parameter.min
                val max = counterOverrides?.max ?: parameter.max
                val value = parameter.defaultValue?.toIntOrNull() ?: min ?: 0

                RemoteAdjustment.Counter(
                    id = "${command.id}:${parameter.name}",
                    label = entry.labelOverride ?: parameter.name,
                    isEnabled = true,
                    value = value,
                    min = min,
                    max = max,
                    helperText = helperText
                )
            }

            AdjustmentControl.AUTO -> null
        }
    }

    private fun mapTelemetryEntry(
        entry: RemoteLayoutEntry.TelemetryEntry,
        command: Command
    ): RemoteTelemetryCard {
        val title = entry.titleOverride ?: command.name

        return when (entry.display) {
            TelemetryDisplay.COUNTER -> RemoteTelemetryCard.Counter(
                id = entry.id,
                title = title,
                isLoading = true,
                lastUpdatedTimestamp = null,
                value = 0,
                unit = entry.unit,
                delta = null
            )

            TelemetryDisplay.GAUGE -> {
                val min = entry.min ?: 0f
                val max = entry.max?.takeIf { it > min } ?: 100f

                RemoteTelemetryCard.Gauge(
                    id = entry.id,
                    title = title,
                    isLoading = true,
                    lastUpdatedTimestamp = null,
                    value = min,
                    min = min,
                    max = max,
                    unit = entry.unit,
                    trend = null
                )
            }

            TelemetryDisplay.TEXT -> RemoteTelemetryCard.Text(
                id = entry.id,
                title = title,
                isLoading = true,
                lastUpdatedTimestamp = null,
                body = entry.textTemplate
                    ?: command.description.takeIf { it.isNotBlank() }
                    ?: ""
            )
        }
    }

    private fun parseBoolean(value: String?): Boolean? {
        if (value == null) return null
        return when {
            value.equals("true", ignoreCase = true) -> true
            value.equals("false", ignoreCase = true) -> false
            value == "1" -> true
            value == "0" -> false
            else -> null
        }
    }

    private suspend fun collectTelemetry(serverId: String, id: String, source: TelemetrySource) {
        while (currentCoroutineContext().isActive) {
            try {
                val card = fetchTelemetryCard(serverId, id, source)
                if (card != null) {
                    updateTelemetryCard(id, card)
                }
            } catch (e: Exception) {
                Timber.w(e, "Telemetry collection failed for ${source.command.id}")
            }
            delay(TELEMETRY_POLL_INTERVAL_MS)
        }
    }

    private suspend fun fetchTelemetryCard(
        serverId: String,
        id: String,
        source: TelemetrySource
    ): RemoteTelemetryCard? {
        val outputBuilder = StringBuilder()
        var exitCode = 0

        try {
            commandRepository.executeCommand(serverId, source.command.id, emptyMap())
                .collect { result ->
                    when (result) {
                        is CommandExecutionResult.Output -> if (!result.isError) {
                            outputBuilder.appendLine(result.text)
                        }

                        is CommandExecutionResult.ExitCode -> exitCode = result.code
                        is CommandExecutionResult.Error -> throw IllegalStateException(result.message)
                    }
                }
        } catch (e: Exception) {
            Timber.w(e, "Telemetry command execution failed: ${source.command.id}")
            return null
        }

        if (exitCode != 0) {
            Timber.w("Telemetry command ${source.command.id} exited with $exitCode")
        }

        val output = outputBuilder.toString().trim()
        if (output.isEmpty() && source.entry.display != TelemetryDisplay.TEXT) {
            Timber.d("Telemetry command ${source.command.id} returned empty output")
        }

        return buildTelemetryCardFromOutput(id, source, output)
    }

    private fun buildTelemetryCardFromOutput(
        id: String,
        source: TelemetrySource,
        output: String
    ): RemoteTelemetryCard? {
        val timestamp = System.currentTimeMillis()
        val previous = telemetryLatestCards[id]

        return when (source.entry.display) {
            TelemetryDisplay.COUNTER -> {
                val numeric = extractNumber(output)?.toLongOrNull() ?: return null
                val delta = (previous as? RemoteTelemetryCard.Counter)?.let { numeric - it.value }
                RemoteTelemetryCard.Counter(
                    id = id,
                    title = source.entry.titleOverride ?: source.command.name,
                    isLoading = false,
                    lastUpdatedTimestamp = timestamp,
                    value = numeric,
                    unit = source.entry.unit,
                    delta = delta
                )
            }

            TelemetryDisplay.GAUGE -> {
                val numeric = extractNumber(output)?.toFloatOrNull() ?: return null
                val min = source.entry.min ?: 0f
                val max = source.entry.max?.takeIf { it > min } ?: 100f
                val clamped = numeric.coerceIn(min, max)
                val previousGauge = previous as? RemoteTelemetryCard.Gauge
                val diff = previousGauge?.let { clamped - it.value } ?: 0f
                val trend = previousGauge?.let {
                    when {
                        diff > 0.5f -> RemoteTelemetryCard.Trend(
                            direction = RemoteTelemetryCard.Trend.Direction.UP,
                            magnitude = abs(diff)
                        )

                        diff < -0.5f -> RemoteTelemetryCard.Trend(
                            direction = RemoteTelemetryCard.Trend.Direction.DOWN,
                            magnitude = abs(diff)
                        )

                        else -> RemoteTelemetryCard.Trend(
                            direction = RemoteTelemetryCard.Trend.Direction.STEADY,
                            magnitude = abs(diff)
                        )
                    }
                }

                RemoteTelemetryCard.Gauge(
                    id = id,
                    title = source.entry.titleOverride ?: source.command.name,
                    isLoading = false,
                    lastUpdatedTimestamp = timestamp,
                    value = clamped,
                    min = min,
                    max = max,
                    unit = source.entry.unit,
                    trend = trend
                )
            }

            TelemetryDisplay.TEXT -> {
                val body = when {
                    source.entry.textTemplate != null ->
                        source.entry.textTemplate.replace("{value}", output.ifEmpty { "—" })

                    output.isNotEmpty() -> output
                    else -> source.command.description
                }.ifEmpty { "—" }

                RemoteTelemetryCard.Text(
                    id = id,
                    title = source.entry.titleOverride ?: source.command.name,
                    isLoading = false,
                    lastUpdatedTimestamp = timestamp,
                    body = body
                )
            }
        }
    }

    private fun extractNumber(output: String): String? {
        return numberRegex.find(output)?.value
    }

    private fun updateTelemetryCard(id: String, card: RemoteTelemetryCard) {
        telemetryLatestCards[id] = card
        val currentState = _uiState.value
        if (currentState is CommandListUiState.Success) {
            val updatedPanel = currentState.remotePanel.copy(
                telemetry = currentState.remotePanel.telemetry.map { existing ->
                    if (existing.id == id) card else existing
                }
            )
            _uiState.value = currentState.copy(remotePanel = updatedPanel)
        }
    }

    private data class TelemetrySource(
        val command: Command,
        val entry: RemoteLayoutEntry.TelemetryEntry
    )

    companion object {
        private const val TELEMETRY_POLL_INTERVAL_MS = 5_000L
        private val numberRegex = Regex("[-+]?[0-9]*\\.?[0-9]+")
    }
}

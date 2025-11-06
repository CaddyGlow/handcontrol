package com.handcontrol.feature.commands.shell

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Cancel
import androidx.compose.material.icons.filled.Keyboard
import androidx.compose.material.icons.filled.Send
import androidx.compose.material3.AssistChip
import androidx.compose.material3.AssistChipDefaults
import androidx.compose.material3.Button
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.hilt.navigation.compose.hiltViewModel
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.handcontrol.data.commands.Command
import com.handcontrol.data.commands.ParameterInputState
import com.handcontrol.ui.components.parameters.ParameterInput

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ShellSessionScreen(
    serverId: String,
    commandId: String,
    onNavigateBack: () -> Unit,
    modifier: Modifier = Modifier,
    viewModel: ShellSessionViewModel = hiltViewModel()
) {
    val uiState by viewModel.uiState.collectAsStateWithLifecycle()
    val snackbarHostState = remember { SnackbarHostState() }
    val outputListState = rememberLazyListState()
    var inputValue by rememberSaveable { mutableStateOf("") }

    LaunchedEffect(serverId, commandId) {
        viewModel.load(serverId, commandId)
    }

    LaunchedEffect(uiState.outputLines.size) {
        if (uiState.outputLines.isNotEmpty()) {
            outputListState.animateScrollToItem(uiState.outputLines.lastIndex)
        }
    }

    LaunchedEffect(Unit) {
        viewModel.toastMessages.collect { message ->
            snackbarHostState.showSnackbar(message)
        }
    }

    val errorMessage = uiState.errorMessage

    Scaffold(
        modifier = modifier,
        snackbarHost = { SnackbarHost(snackbarHostState) },
        topBar = {
            TopAppBar(
                title = {
                    Text(
                        text = uiState.command?.name ?: "Interactive Shell",
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis
                    )
                },
                navigationIcon = {
                    IconButton(onClick = onNavigateBack) {
                        Icon(
                            imageVector = Icons.AutoMirrored.Filled.ArrowBack,
                            contentDescription = "Back"
                        )
                    }
                },
                actions = {
                    if (uiState.connectionState is ShellConnectionState.Active ||
                        uiState.connectionState is ShellConnectionState.Connecting
                    ) {
                        IconButton(onClick = { viewModel.closeSession("Client closed") }) {
                            Icon(
                                imageVector = Icons.Filled.Cancel,
                                contentDescription = "Close session"
                            )
                        }
                    }
                }
            )
        }
    ) { paddingValues ->
        when {
            uiState.isLoading -> {
                LoadingShellContent(Modifier.padding(paddingValues))
            }
            errorMessage != null && uiState.command == null -> {
                ErrorShellContent(
                    message = errorMessage,
                    onRetry = { viewModel.load(serverId, commandId) },
                    modifier = Modifier.padding(paddingValues)
                )
            }
            else -> {
                ShellSessionContent(
                    uiState = uiState,
                    onUpdateParameter = viewModel::updateParameter,
                    onRefreshDefault = viewModel::refreshDefault,
                    onStartSession = viewModel::startSession,
                    onSendInput = { text ->
                        viewModel.sendText(text)
                        inputValue = ""
                    },
                    onSendCtrlC = viewModel::sendCtrlC,
                    onCloseSession = viewModel::closeSession,
                    outputListState = outputListState,
                    inputValue = inputValue,
                    onInputChange = { inputValue = it },
                    modifier = Modifier.padding(paddingValues)
                )
            }
        }
    }
}

@Composable
private fun ShellSessionContent(
    uiState: ShellSessionUiState,
    onUpdateParameter: (String, String) -> Unit,
    onRefreshDefault: (String) -> Unit,
    onStartSession: () -> Unit,
    onSendInput: (String) -> Unit,
    onSendCtrlC: () -> Unit,
    onCloseSession: (String?) -> Unit,
    outputListState: androidx.compose.foundation.lazy.LazyListState,
    inputValue: String,
    onInputChange: (String) -> Unit,
    modifier: Modifier = Modifier
) {
    Column(
        modifier = modifier
            .fillMaxSize()
            .padding(horizontal = 16.dp, vertical = 12.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp)
    ) {
        uiState.command?.let { command ->
            Text(
                text = command.description,
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant
            )

            val showStartControls =
                uiState.connectionState !is ShellConnectionState.Active &&
                    uiState.connectionState !is ShellConnectionState.Connecting

            if (showStartControls) {
                if (command.parameters.isNotEmpty()) {
                    ParameterSection(
                        command = command,
                        parameterStates = uiState.parameterStates,
                        dynamicLoading = uiState.dynamicLoading,
                        onUpdateParameter = onUpdateParameter,
                        onRefreshDefault = onRefreshDefault
                    )
                }

                val startLabel = when (uiState.connectionState) {
                    is ShellConnectionState.Completed -> "Restart Session"
                    is ShellConnectionState.Failed -> "Retry Session"
                    is ShellConnectionState.Closed -> "Start Session"
                    else -> "Start Session"
                }

                Button(
                    onClick = onStartSession,
                    enabled = uiState.isFormValid,
                    modifier = Modifier.fillMaxWidth()
                ) {
                    Text(startLabel)
                }
            }
        }

        ConnectionStatus(uiState.connectionState)

        SessionOutputList(
            lines = uiState.outputLines,
            state = outputListState,
            modifier = Modifier
                .weight(1f)
                .fillMaxWidth()
                .background(
                    color = MaterialTheme.colorScheme.surfaceVariant,
                    shape = RoundedCornerShape(12.dp)
                )
        )

        when (uiState.connectionState) {
            is ShellConnectionState.Active -> {
                InputToolbar(
                    inputValue = inputValue,
                    onInputChange = onInputChange,
                    onSendInput = onSendInput,
                    onSendCtrlC = onSendCtrlC,
                    onCloseSession = onCloseSession
                )
            }
            is ShellConnectionState.Connecting -> {
                LinearProgressIndicator(
                    modifier = Modifier.fillMaxWidth()
                )
            }
            else -> Unit
        }
    }
}

@Composable
private fun ParameterSection(
    command: Command,
    parameterStates: Map<String, ParameterInputState>,
    dynamicLoading: Map<String, Boolean>,
    onUpdateParameter: (String, String) -> Unit,
    onRefreshDefault: (String) -> Unit
) {
    Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
        command.parameters.forEach { param ->
            val state = parameterStates[param.name] ?: return@forEach
            ParameterInput(
                parameter = state.parameter,
                inputState = state,
                onValueChange = { value -> onUpdateParameter(param.name, value) },
                onRefreshDefault = if (param.defaultValueCommand != null) {
                    { onRefreshDefault(param.name) }
                } else null,
                isLoadingDefault = dynamicLoading[param.name] == true
            )
        }
    }
}

@Composable
private fun SessionOutputList(
    lines: List<ShellLine>,
    state: androidx.compose.foundation.lazy.LazyListState,
    modifier: Modifier = Modifier
) {
    LazyColumn(
        state = state,
        modifier = modifier
            .padding(12.dp)
    ) {
        itemsIndexed(lines) { _, line ->
            val color = when {
                line.isError -> MaterialTheme.colorScheme.error
                line.isSystem -> MaterialTheme.colorScheme.secondary
                else -> MaterialTheme.colorScheme.onSurface
            }
            Text(
                text = line.text,
                color = color,
                style = MaterialTheme.typography.bodySmall,
                fontFamily = FontFamily.Monospace,
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(vertical = 2.dp)
            )
        }
    }
}

@Composable
private fun ConnectionStatus(state: ShellConnectionState) {
    val (label, color) = when (state) {
        is ShellConnectionState.NotStarted -> "Session not started" to MaterialTheme.colorScheme.outline
        is ShellConnectionState.Connecting -> "Connecting…" to MaterialTheme.colorScheme.primary
        is ShellConnectionState.Active -> {
            val message = state.readyMessage?.takeIf { it.isNotBlank() } ?: "Connected"
            message to MaterialTheme.colorScheme.primary
        }
        is ShellConnectionState.Completed -> {
            val message = if (state.exitCode == 0) {
                "Exited successfully"
            } else {
                "Exited (${state.exitCode})"
            }
            message to MaterialTheme.colorScheme.tertiary
        }
        is ShellConnectionState.Failed -> state.message to MaterialTheme.colorScheme.error
        is ShellConnectionState.Closed -> {
            val reason = state.reason?.takeIf { it.isNotBlank() }
            val message = reason?.let { "Closed: $it" } ?: "Closed"
            message to MaterialTheme.colorScheme.outline
        }
    }

    AssistChip(
        onClick = {},
        label = { Text(label) },
        colors = AssistChipDefaults.assistChipColors(
            containerColor = color.copy(alpha = 0.16f),
            labelColor = color
        ),
        leadingIcon = {
            Icon(
                imageVector = Icons.Filled.Keyboard,
                contentDescription = null,
                modifier = Modifier.size(16.dp),
                tint = color
            )
        }
    )
}

@Composable
private fun InputToolbar(
    inputValue: String,
    onInputChange: (String) -> Unit,
    onSendInput: (String) -> Unit,
    onSendCtrlC: () -> Unit,
    onCloseSession: (String?) -> Unit
) {
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp)
        ) {
            OutlinedTextField(
                value = inputValue,
                onValueChange = onInputChange,
                modifier = Modifier.weight(1f),
                label = { Text("Send input") },
                singleLine = true
            )
            Button(
                onClick = { onSendInput(inputValue) },
                enabled = inputValue.isNotBlank()
            ) {
                Icon(imageVector = Icons.Filled.Send, contentDescription = null)
                Spacer(modifier = Modifier.width(4.dp))
                Text("Send")
            }
        }
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp)
        ) {
            OutlinedButton(onClick = onSendCtrlC) {
                Text("Ctrl+C")
            }
            Spacer(modifier = Modifier.weight(1f))
            TextButton(onClick = { onCloseSession("Client closed") }) {
                Text("Close Session")
            }
        }
    }
}

@Composable
private fun LoadingShellContent(modifier: Modifier = Modifier) {
    Box(
        modifier = modifier
            .fillMaxSize(),
        contentAlignment = Alignment.Center
    ) {
        Column(horizontalAlignment = Alignment.CenterHorizontally) {
            LinearProgressIndicator(modifier = Modifier.fillMaxWidth(0.6f))
            Spacer(modifier = Modifier.height(16.dp))
            Text("Loading shell capability…")
        }
    }
}

@Composable
private fun ErrorShellContent(
    message: String,
    onRetry: () -> Unit,
    modifier: Modifier = Modifier
) {
    Box(
        modifier = modifier.fillMaxSize(),
        contentAlignment = Alignment.Center
    ) {
        Column(
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(12.dp)
        ) {
            Text(
                text = message,
                style = MaterialTheme.typography.bodyLarge,
                color = MaterialTheme.colorScheme.error,
                textAlign = androidx.compose.ui.text.style.TextAlign.Center
            )
            Button(onClick = onRetry) {
                Text("Retry")
            }
        }
    }
}

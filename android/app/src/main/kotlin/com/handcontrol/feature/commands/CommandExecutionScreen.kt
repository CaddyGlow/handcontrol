package com.handcontrol.feature.commands

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.CheckCircle
import androidx.compose.material.icons.filled.Error
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.tooling.preview.Preview
import androidx.compose.ui.unit.dp
import androidx.hilt.navigation.compose.hiltViewModel
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewModelScope
import com.handcontrol.data.commands.defaultValueFor
import com.handcontrol.data.commands.toProtoParameter
import com.handcontrol.data.commands.validateParameterValue
import com.handcontrol.ui.theme.HandControlTheme
import kotlinx.coroutines.launch

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun CommandExecutionScreen(
    serverId: String,
    commandId: String,
    onNavigateBack: () -> Unit,
    modifier: Modifier = Modifier,
    viewModel: CommandListViewModel = hiltViewModel()
) {
    val listUiState by viewModel.uiState.collectAsStateWithLifecycle()
    val executionState by viewModel.executionState.collectAsStateWithLifecycle()
    val listState = rememberLazyListState()

    // Parameter state management
    var parameterStates by remember { mutableStateOf<Map<String, com.handcontrol.data.commands.ParameterInputState>>(emptyMap()) }
    var isFormValid by remember { mutableStateOf(false) }
    var dynamicDefaultLoadingStates by remember { mutableStateOf<Map<String, Boolean>>(emptyMap()) }

    LaunchedEffect(Unit) {
        viewModel.loadCommands(serverId)
    }

    val command = when (val state = listUiState) {
        is CommandListUiState.Success -> state.commands.find { it.id == commandId }
        else -> null
    }

    // Initialize parameter states and fetch dynamic defaults when command is loaded
    LaunchedEffect(command) {
        command?.let { cmd ->
            if (cmd.parameters.isNotEmpty()) {
                // Initialize with static defaults first
                parameterStates = cmd.parameters.associate { param ->
                    val protoParam = param.toProtoParameter()
                    param.name to com.handcontrol.data.commands.ParameterInputState(
                        parameter = protoParam,
                        currentValue = param.defaultValue ?: defaultValueFor(param.type),
                        validationResult = com.handcontrol.data.commands.ValidationResult.Pending,
                        isDirty = false
                    )
                }

                // Fetch dynamic defaults for parameters that have them
                cmd.parameters.forEach { param ->
                    if (param.defaultValueCommand != null) {
                        dynamicDefaultLoadingStates = dynamicDefaultLoadingStates + (param.name to true)

                        viewModel.viewModelScope.launch {
                            val value = viewModel.fetchDynamicDefault(
                                command = param.defaultValueCommand,
                                pattern = param.defaultValuePattern,
                                fallback = param.defaultValue ?: defaultValueFor(param.type)
                            )

                            // Update parameter state with fetched value
                            val currentState = parameterStates[param.name]
                            if (currentState != null) {
                                parameterStates = parameterStates + (param.name to currentState.copy(
                                    currentValue = value,
                                    validationResult = validateParameterValue(currentState.parameter, value)
                                ))
                            }

                            dynamicDefaultLoadingStates = dynamicDefaultLoadingStates + (param.name to false)
                        }
                    }
                }
            }
        }
    }

    // Validate form
    LaunchedEffect(parameterStates) {
        isFormValid = parameterStates.values.all {
            it.validationResult is com.handcontrol.data.commands.ValidationResult.Valid
        }
    }

    Scaffold(
        topBar = {
            TopAppBar(
                title = {
                    Text(command?.name ?: "Execute Command")
                },
                navigationIcon = {
                    IconButton(onClick = onNavigateBack) {
                        Icon(
                            imageVector = Icons.AutoMirrored.Filled.ArrowBack,
                            contentDescription = "Back"
                        )
                    }
                }
            )
        }
    ) { paddingValues ->
        Column(
            modifier = modifier
                .fillMaxSize()
                .padding(paddingValues)
        ) {
            when (val state = executionState) {
                is CommandExecutionState.Idle -> {
                    command?.let { cmd ->
                        if (cmd.parameters.isEmpty()) {
                            // Simple command without parameters
                            ReadyToExecuteView(
                                commandName = cmd.name,
                                commandDescription = cmd.description,
                                onExecute = {
                                    viewModel.triggerQuickCommand(cmd)
                                }
                            )
                        } else {
                            // Command with parameters
                            ReadyToExecuteWithParametersView(
                                command = cmd,
                                parameterStates = parameterStates,
                                dynamicDefaultLoadingStates = dynamicDefaultLoadingStates,
                                onParameterChange = { paramName, value ->
                                    val currentState = parameterStates[paramName] ?: return@ReadyToExecuteWithParametersView
                                    val validationResult = validateParameterValue(currentState.parameter, value)
                                    parameterStates = parameterStates + (paramName to currentState.copy(
                                        currentValue = value,
                                        validationResult = validationResult,
                                        isDirty = true
                                    ))
                                },
                                onRefreshDefault = { paramName ->
                                    val param = cmd.parameters.find { it.name == paramName }
                                    if (param?.defaultValueCommand != null) {
                                        dynamicDefaultLoadingStates = dynamicDefaultLoadingStates + (paramName to true)

                                        viewModel.viewModelScope.launch {
                                            val value = viewModel.fetchDynamicDefault(
                                                command = param.defaultValueCommand,
                                                pattern = param.defaultValuePattern,
                                                fallback = param.defaultValue ?: defaultValueFor(param.type)
                                            )

                                            val currentState = parameterStates[paramName]
                                            if (currentState != null) {
                                                parameterStates = parameterStates + (paramName to currentState.copy(
                                                    currentValue = value,
                                                    validationResult = validateParameterValue(currentState.parameter, value)
                                                ))
                                            }

                                            dynamicDefaultLoadingStates = dynamicDefaultLoadingStates + (paramName to false)
                                        }
                                    }
                                },
                                onExecute = {
                                    val params = parameterStates.mapValues { it.value.currentValue }
                                    viewModel.triggerQuickCommand(cmd, params)
                                    // Navigate back immediately if command doesn't show output
                                    if (!cmd.showOutput) {
                                        onNavigateBack()
                                    }
                                },
                                canExecute = isFormValid
                            )
                        }
                    } ?: LoadingCommandView()
                }

                is CommandExecutionState.Executing -> {
                    ExecutionOutputView(
                        output = state.output,
                        isExecuting = true,
                        exitCode = null,
                        error = null,
                        onRetry = {},
                        listState = listState
                    )
                }

                is CommandExecutionState.Completed -> {
                    ExecutionOutputView(
                        output = state.output,
                        isExecuting = false,
                        exitCode = state.exitCode,
                        error = null,
                        onRetry = {
                            viewModel.clearExecution()
                        },
                        listState = listState
                    )
                }

                is CommandExecutionState.Failed -> {
                    ExecutionErrorView(
                        message = state.message,
                        onRetry = {
                            viewModel.clearExecution()
                        },
                        onBack = onNavigateBack
                    )
                }
            }
        }
    }

    LaunchedEffect(executionState) {
        if (executionState is CommandExecutionState.Executing) {
            val output = (executionState as CommandExecutionState.Executing).output
            if (output.isNotEmpty()) {
                listState.animateScrollToItem(output.size - 1)
            }
        }
    }
}

@Composable
private fun LoadingCommandView(modifier: Modifier = Modifier) {
    Column(
        modifier = modifier.fillMaxSize(),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.Center
    ) {
        CircularProgressIndicator(
            modifier = Modifier.size(48.dp),
            color = MaterialTheme.colorScheme.primary
        )
        Spacer(modifier = Modifier.height(16.dp))
        Text(
            text = "Loading command...",
            style = MaterialTheme.typography.bodyLarge,
            color = MaterialTheme.colorScheme.onSurfaceVariant
        )
    }
}

@Composable
private fun ReadyToExecuteView(
    commandName: String,
    commandDescription: String,
    onExecute: () -> Unit,
    modifier: Modifier = Modifier
) {
    Column(
        modifier = modifier
            .fillMaxSize()
            .padding(24.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.Center
    ) {
        Text(
            text = commandName,
            style = MaterialTheme.typography.headlineMedium,
            color = MaterialTheme.colorScheme.onSurface,
            textAlign = TextAlign.Center
        )

        Spacer(modifier = Modifier.height(16.dp))

        Text(
            text = commandDescription,
            style = MaterialTheme.typography.bodyLarge,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            textAlign = TextAlign.Center
        )

        Spacer(modifier = Modifier.height(48.dp))

        Button(
            onClick = onExecute,
            modifier = Modifier.fillMaxWidth()
        ) {
            Text("Execute Command")
        }
    }
}

@Composable
private fun ReadyToExecuteWithParametersView(
    command: com.handcontrol.data.commands.Command,
    parameterStates: Map<String, com.handcontrol.data.commands.ParameterInputState>,
    dynamicDefaultLoadingStates: Map<String, Boolean>,
    onParameterChange: (String, String) -> Unit,
    onRefreshDefault: (String) -> Unit,
    onExecute: () -> Unit,
    canExecute: Boolean,
    modifier: Modifier = Modifier
) {
    Column(
        modifier = modifier
            .fillMaxSize()
            .padding(16.dp)
    ) {
        // Command description
        if (command.description.isNotBlank()) {
            Card(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(bottom = 16.dp),
                colors = CardDefaults.cardColors(
                    containerColor = MaterialTheme.colorScheme.primaryContainer
                )
            ) {
                Text(
                    text = command.description,
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onPrimaryContainer,
                    modifier = Modifier.padding(16.dp)
                )
            }
        }

        // Parameters section
        Text(
            text = "Parameters",
            style = MaterialTheme.typography.titleMedium,
            modifier = Modifier.padding(vertical = 8.dp)
        )

        Column(modifier = Modifier.weight(1f)) {
            command.parameters.forEach { parameter ->
                parameterStates[parameter.name]?.let { inputState ->
                    com.handcontrol.ui.components.parameters.ParameterInput(
                        parameter = parameter.toProtoParameter(),
                        inputState = inputState,
                        onValueChange = { value ->
                            onParameterChange(parameter.name, value)
                        },
                        onRefreshDefault = if (parameter.defaultValueCommand != null) {
                            { onRefreshDefault(parameter.name) }
                        } else null,
                        isLoadingDefault = dynamicDefaultLoadingStates[parameter.name] ?: false
                    )
                }
            }
        }

        // Execute button
        Button(
            onClick = onExecute,
            enabled = canExecute,
            modifier = Modifier.fillMaxWidth()
        ) {
            Text("Execute Command")
        }
    }
}

@Composable
private fun ExecutionOutputView(
    output: List<CommandOutputLine>,
    isExecuting: Boolean,
    exitCode: Int?,
    error: String?,
    onRetry: () -> Unit,
    listState: androidx.compose.foundation.lazy.LazyListState,
    modifier: Modifier = Modifier
) {
    Column(modifier = modifier.fillMaxSize()) {
        Card(
            modifier = Modifier
                .fillMaxWidth()
                .padding(16.dp),
            colors = CardDefaults.cardColors(
                containerColor = when {
                    error != null -> MaterialTheme.colorScheme.errorContainer
                    exitCode != null && exitCode == 0 -> MaterialTheme.colorScheme.primaryContainer
                    exitCode != null && exitCode != 0 -> MaterialTheme.colorScheme.errorContainer
                    else -> MaterialTheme.colorScheme.surfaceVariant
                }
            )
        ) {
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(16.dp),
                verticalAlignment = Alignment.CenterVertically
            ) {
                when {
                    isExecuting -> {
                        CircularProgressIndicator(
                            modifier = Modifier.size(24.dp),
                            strokeWidth = 3.dp,
                            color = MaterialTheme.colorScheme.onSurfaceVariant
                        )
                        Spacer(modifier = Modifier.width(12.dp))
                        Text(
                            text = "Executing...",
                            style = MaterialTheme.typography.titleMedium,
                            color = MaterialTheme.colorScheme.onSurfaceVariant
                        )
                    }

                    exitCode != null -> {
                        if (exitCode == 0) {
                            Icon(
                                imageVector = Icons.Default.CheckCircle,
                                contentDescription = null,
                                tint = MaterialTheme.colorScheme.onPrimaryContainer,
                                modifier = Modifier.size(24.dp)
                            )
                            Spacer(modifier = Modifier.width(12.dp))
                            Text(
                                text = "Completed Successfully",
                                style = MaterialTheme.typography.titleMedium,
                                color = MaterialTheme.colorScheme.onPrimaryContainer
                            )
                        } else {
                            Icon(
                                imageVector = Icons.Default.Error,
                                contentDescription = null,
                                tint = MaterialTheme.colorScheme.onErrorContainer,
                                modifier = Modifier.size(24.dp)
                            )
                            Spacer(modifier = Modifier.width(12.dp))
                            Text(
                                text = "Exit Code: $exitCode",
                                style = MaterialTheme.typography.titleMedium,
                                color = MaterialTheme.colorScheme.onErrorContainer
                            )
                        }
                    }
                }
            }
        }

        LazyColumn(
            state = listState,
            modifier = Modifier
                .weight(1f)
                .fillMaxWidth()
                .background(MaterialTheme.colorScheme.surfaceVariant)
                .padding(horizontal = 16.dp),
            contentPadding = PaddingValues(vertical = 8.dp)
        ) {
            items(output) { line ->
                Text(
                    text = line.text,
                    style = MaterialTheme.typography.bodySmall,
                    fontFamily = FontFamily.Monospace,
                    color = if (line.isError) {
                        MaterialTheme.colorScheme.error
                    } else {
                        MaterialTheme.colorScheme.onSurfaceVariant
                    },
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(vertical = 2.dp)
                )
            }

            if (output.isEmpty() && isExecuting) {
                item {
                    Text(
                        text = "Waiting for output...",
                        style = MaterialTheme.typography.bodySmall,
                        fontFamily = FontFamily.Monospace,
                        color = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.6f)
                    )
                }
            }
        }

        if (exitCode != null) {
            OutlinedButton(
                onClick = onRetry,
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(16.dp)
            ) {
                Text("Run Again")
            }
        }
    }
}

@Composable
private fun ExecutionErrorView(
    message: String,
    onRetry: () -> Unit,
    onBack: () -> Unit,
    modifier: Modifier = Modifier
) {
    Column(
        modifier = modifier
            .fillMaxSize()
            .padding(24.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.Center
    ) {
        Icon(
            imageVector = Icons.Default.Error,
            contentDescription = null,
            tint = MaterialTheme.colorScheme.error,
            modifier = Modifier.size(80.dp)
        )

        Spacer(modifier = Modifier.height(24.dp))

        Text(
            text = "Execution Failed",
            style = MaterialTheme.typography.headlineMedium,
            color = MaterialTheme.colorScheme.error,
            textAlign = TextAlign.Center
        )

        Spacer(modifier = Modifier.height(16.dp))

        Text(
            text = message,
            style = MaterialTheme.typography.bodyLarge,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            textAlign = TextAlign.Center
        )

        Spacer(modifier = Modifier.height(48.dp))

        Button(
            onClick = onRetry,
            modifier = Modifier.fillMaxWidth()
        ) {
            Text("Try Again")
        }

        Spacer(modifier = Modifier.height(16.dp))

        OutlinedButton(
            onClick = onBack,
            modifier = Modifier.fillMaxWidth()
        ) {
            Text("Back to Commands")
        }
    }
}

@Preview(showBackground = true, showSystemUi = true)
@Composable
private fun ReadyToExecutePreview() {
    HandControlTheme {
        ReadyToExecuteView(
            commandName = "System Info",
            commandDescription = "Display detailed system information",
            onExecute = {}
        )
    }
}

@Preview(showBackground = true, showSystemUi = true)
@Composable
private fun ExecutionOutputPreview() {
    HandControlTheme {
        ExecutionOutputView(
            output = listOf(
                CommandOutputLine("Starting system information collection...", false),
                CommandOutputLine("OS: Linux 6.1.0", false),
                CommandOutputLine("Kernel: x86_64", false),
                CommandOutputLine("CPU: Intel Core i7-10700K @ 3.80GHz", false),
                CommandOutputLine("RAM: 32GB", false),
                CommandOutputLine("Disk: 1TB SSD", false),
                CommandOutputLine("Network: Active", false),
                CommandOutputLine("Done.", false)
            ),
            isExecuting = false,
            exitCode = 0,
            error = null,
            onRetry = {},
            listState = rememberLazyListState()
        )
    }
}

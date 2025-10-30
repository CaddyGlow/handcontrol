package com.handcontrol.feature.commands

import androidx.compose.foundation.clickable
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
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.automirrored.filled.List
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.Computer
import androidx.compose.material.icons.filled.Error
import androidx.compose.material.icons.filled.PlayArrow
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material.icons.filled.Search
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.tooling.preview.Preview
import androidx.compose.ui.unit.dp
import androidx.hilt.navigation.compose.hiltViewModel
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.handcontrol.data.commands.Command
import com.handcontrol.data.commands.ParameterType
import com.handcontrol.ui.components.CommandConfirmationDialog
import com.handcontrol.ui.theme.HandControlTheme
import kotlinx.coroutines.flow.collectLatest

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun CommandListScreen(
    serverHost: String,
    serverPort: Int,
    onNavigateToCommandExecution: (String, Int, String) -> Unit,
    onNavigateBack: () -> Unit = {},
    onNavigateToServerList: () -> Unit = {},
    modifier: Modifier = Modifier,
    viewModel: CommandListViewModel = hiltViewModel()
) {
    val uiState by viewModel.uiState.collectAsStateWithLifecycle()
    val snackbarHostState = remember { SnackbarHostState() }

    var showConfirmationDialog by remember { mutableStateOf(false) }
    var selectedCommand by remember { mutableStateOf<Command?>(null) }

    LaunchedEffect(Unit) {
        viewModel.loadCommands(serverHost, serverPort)
    }

    // Collect toast messages
    LaunchedEffect(Unit) {
        viewModel.toastMessage.collectLatest { message ->
            snackbarHostState.showSnackbar(message)
        }
    }

    Scaffold(
        snackbarHost = { SnackbarHost(snackbarHostState) },
        topBar = {
            TopAppBar(
                title = {
                    when (val state = uiState) {
                        is CommandListUiState.Success -> {
                            Column {
                                Text("Commands")
                                Text(
                                    text = "${state.serverInfo.hostname} (${state.serverInfo.os})",
                                    style = MaterialTheme.typography.bodySmall,
                                    color = MaterialTheme.colorScheme.onSurfaceVariant
                                )
                            }
                        }
                        else -> Text("Commands")
                    }
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
                    IconButton(onClick = onNavigateToServerList) {
                        Icon(
                            imageVector = Icons.AutoMirrored.Filled.List,
                            contentDescription = "Server List"
                        )
                    }
                    IconButton(onClick = {
                        viewModel.loadCommands(serverHost, serverPort)
                    }) {
                        Icon(
                            imageVector = Icons.Default.Refresh,
                            contentDescription = "Refresh"
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
            when (val state = uiState) {
                is CommandListUiState.Loading -> {
                    LoadingView()
                }

                is CommandListUiState.Error -> {
                    ErrorView(
                        message = state.message,
                        onRetry = {
                            viewModel.loadCommands(serverHost, serverPort)
                        }
                    )
                }

                is CommandListUiState.Success -> {
                    if (state.commands.isEmpty()) {
                        EmptyCommandsView()
                    } else {
                        CommandListContent(
                            commands = state.filteredCommands,
                            searchQuery = state.searchQuery,
                            onSearchQueryChange = { query ->
                                viewModel.searchCommands(query)
                            },
                            onCommandClick = { command ->
                                handleCommandClick(
                                    command = command,
                                    viewModel = viewModel,
                                    onNavigateToExecution = {
                                        onNavigateToCommandExecution(serverHost, serverPort, command.id)
                                    },
                                    onShowConfirmation = {
                                        selectedCommand = command
                                        showConfirmationDialog = true
                                    }
                                )
                            }
                        )
                    }
                }
            }
        }
    }

    // Confirmation dialog
    if (showConfirmationDialog && selectedCommand != null) {
        CommandConfirmationDialog(
            command = selectedCommand!!,
            onConfirm = {
                val cmd = selectedCommand!!
                viewModel.executeCommandWithMode(
                    commandId = cmd.id,
                    commandName = cmd.name,
                    parameters = emptyMap(),
                    showOutput = cmd.showOutput
                )
                if (cmd.showOutput) {
                    onNavigateToCommandExecution(serverHost, serverPort, cmd.id)
                }
                showConfirmationDialog = false
                selectedCommand = null
            },
            onDismiss = {
                showConfirmationDialog = false
                selectedCommand = null
            }
        )
    }
}

/**
 * Determine how to handle command click based on parameters and flags
 */
private fun handleCommandClick(
    command: Command,
    viewModel: CommandListViewModel,
    onNavigateToExecution: () -> Unit,
    onShowConfirmation: () -> Unit
) {
    when {
        // Has parameters - always show parameter screen
        command.parameters.isNotEmpty() -> {
            onNavigateToExecution()
        }
        // No parameters but requires confirmation
        command.requiresConfirmation -> {
            onShowConfirmation()
        }
        // No parameters, no confirmation - immediate execution
        else -> {
            viewModel.executeCommandWithMode(
                commandId = command.id,
                commandName = command.name,
                parameters = emptyMap(),
                showOutput = command.showOutput
            )
            if (command.showOutput) {
                onNavigateToExecution()
            }
        }
    }
}

@Composable
private fun LoadingView(modifier: Modifier = Modifier) {
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
            text = "Loading commands...",
            style = MaterialTheme.typography.bodyLarge,
            color = MaterialTheme.colorScheme.onSurfaceVariant
        )
    }
}

@Composable
private fun ErrorView(
    message: String,
    onRetry: () -> Unit,
    modifier: Modifier = Modifier
) {
    Column(
        modifier = modifier.fillMaxSize(),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.Center
    ) {
        Icon(
            imageVector = Icons.Default.Error,
            contentDescription = null,
            tint = MaterialTheme.colorScheme.error,
            modifier = Modifier.size(64.dp)
        )

        Spacer(modifier = Modifier.height(16.dp))

        Text(
            text = "Failed to Load Commands",
            style = MaterialTheme.typography.titleLarge,
            color = MaterialTheme.colorScheme.error,
            textAlign = TextAlign.Center
        )

        Spacer(modifier = Modifier.height(8.dp))

        Text(
            text = message,
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            textAlign = TextAlign.Center,
            modifier = Modifier.padding(horizontal = 32.dp)
        )

        Spacer(modifier = Modifier.height(32.dp))

        Button(onClick = onRetry) {
            Text("Retry")
        }
    }
}

@Composable
private fun EmptyCommandsView(modifier: Modifier = Modifier) {
    Column(
        modifier = modifier.fillMaxSize(),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.Center
    ) {
        Icon(
            imageVector = Icons.Default.Computer,
            contentDescription = null,
            tint = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.size(64.dp)
        )

        Spacer(modifier = Modifier.height(16.dp))

        Text(
            text = "No Commands Available",
            style = MaterialTheme.typography.titleLarge,
            color = MaterialTheme.colorScheme.onSurface,
            textAlign = TextAlign.Center
        )

        Spacer(modifier = Modifier.height(8.dp))

        Text(
            text = "This server has no commands configured",
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            textAlign = TextAlign.Center
        )
    }
}

@Composable
private fun CommandListContent(
    commands: List<Command>,
    searchQuery: String,
    onSearchQueryChange: (String) -> Unit,
    onCommandClick: (Command) -> Unit,
    modifier: Modifier = Modifier
) {
    Column(modifier = modifier.fillMaxSize()) {
        OutlinedTextField(
            value = searchQuery,
            onValueChange = onSearchQueryChange,
            modifier = Modifier
                .fillMaxWidth()
                .padding(16.dp),
            placeholder = { Text("Search commands...") },
            leadingIcon = {
                Icon(
                    imageVector = Icons.Default.Search,
                    contentDescription = null
                )
            },
            trailingIcon = {
                if (searchQuery.isNotEmpty()) {
                    IconButton(onClick = { onSearchQueryChange("") }) {
                        Icon(
                            imageVector = Icons.Default.Close,
                            contentDescription = "Clear"
                        )
                    }
                }
            },
            singleLine = true
        )

        if (commands.isEmpty()) {
            Column(
                modifier = Modifier.fillMaxSize(),
                horizontalAlignment = Alignment.CenterHorizontally,
                verticalArrangement = Arrangement.Center
            ) {
                Text(
                    text = "No commands match your search",
                    style = MaterialTheme.typography.bodyLarge,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )
            }
        } else {
            LazyColumn(
                contentPadding = PaddingValues(horizontal = 16.dp, vertical = 8.dp),
                verticalArrangement = Arrangement.spacedBy(12.dp)
            ) {
                items(commands, key = { it.id }) { command ->
                    CommandCard(
                        command = command,
                        onClick = { onCommandClick(command) }
                    )
                }
            }
        }
    }
}

@Composable
private fun CommandCard(
    command: Command,
    onClick: () -> Unit,
    modifier: Modifier = Modifier
) {
    Card(
        modifier = modifier
            .fillMaxWidth()
            .clickable(onClick = onClick),
        colors = CardDefaults.cardColors(
            containerColor = MaterialTheme.colorScheme.surfaceVariant
        )
    ) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(16.dp),
            verticalAlignment = Alignment.CenterVertically
        ) {
            Icon(
                imageVector = Icons.Default.PlayArrow,
                contentDescription = null,
                tint = MaterialTheme.colorScheme.primary,
                modifier = Modifier.size(32.dp)
            )

            Spacer(modifier = Modifier.width(16.dp))

            Column(modifier = Modifier.weight(1f)) {
                Text(
                    text = command.name,
                    style = MaterialTheme.typography.titleMedium,
                    color = MaterialTheme.colorScheme.onSurface,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis
                )

                Spacer(modifier = Modifier.height(4.dp))

                Text(
                    text = command.description,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = 2,
                    overflow = TextOverflow.Ellipsis
                )

                if (command.parameters.isNotEmpty()) {
                    Spacer(modifier = Modifier.height(8.dp))
                    Text(
                        text = "${command.parameters.size} parameter${if (command.parameters.size != 1) "s" else ""}",
                        style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.secondary
                    )
                }
            }
        }
    }
}

@Preview(showBackground = true, showSystemUi = true)
@Composable
private fun CommandListContentPreview() {
    HandControlTheme {
        CommandListContent(
            commands = listOf(
                Command(
                    id = "1",
                    name = "System Info",
                    description = "Display system information",
                    icon = "info",
                    tags = emptyList(),
                    parameters = emptyList()
                ),
                Command(
                    id = "2",
                    name = "Disk Usage",
                    description = "Show disk usage statistics",
                    icon = "storage",
                    tags = emptyList(),
                    parameters = emptyList()
                ),
                Command(
                    id = "3",
                    name = "Custom Script",
                    description = "Run a custom script with parameters",
                    icon = "script",
                    tags = emptyList(),
                    parameters = listOf(
                        com.handcontrol.data.commands.CommandParameter(
                            name = "input",
                            type = ParameterType.TEXT,
                            description = "Input parameter",
                            defaultValue = ""
                        )
                    )
                )
            ),
            searchQuery = "",
            onSearchQueryChange = {},
            onCommandClick = {}
        )
    }
}

@Preview(showBackground = true, showSystemUi = true)
@Composable
private fun EmptyCommandsPreview() {
    HandControlTheme {
        EmptyCommandsView()
    }
}

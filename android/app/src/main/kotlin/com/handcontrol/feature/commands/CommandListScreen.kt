package com.handcontrol.feature.commands

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
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
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.automirrored.filled.List
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.Computer
import androidx.compose.material.icons.filled.Error
import androidx.compose.material.icons.filled.PlayArrow
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material.icons.filled.Search
import androidx.compose.material.icons.filled.Edit
import androidx.compose.material.icons.filled.Save
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FloatingActionButton
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.PrimaryTabRow
import androidx.compose.material3.Tab
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.saveable.rememberSaveable
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
import com.handcontrol.feature.commands.remote.RemoteLayoutBuilderScreen
import com.handcontrol.feature.commands.remote.RemoteLayoutSpec
import com.handcontrol.feature.commands.remote.RemotePanelScreen
import com.handcontrol.feature.commands.remote.RemoteQuickAction

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun CommandListScreen(
    serverId: String,
    onNavigateToCommandExecution: (String, String) -> Unit,
    onNavigateBack: () -> Unit = {},
    onNavigateToServerList: () -> Unit = {},
    modifier: Modifier = Modifier,
    viewModel: CommandListViewModel = hiltViewModel()
) {
    val uiState by viewModel.uiState.collectAsStateWithLifecycle()
    val snackbarHostState = remember { SnackbarHostState() }

    var showConfirmationDialog by remember { mutableStateOf(false) }
    var selectedCommand by remember { mutableStateOf<Command?>(null) }
    var selectedTab by remember { mutableStateOf(CommandListTab.List) }
    var isEditingRemote by rememberSaveable { mutableStateOf(false) }
    var layoutDraft by remember(serverId) { mutableStateOf(RemoteLayoutSpec.withDefaultSections()) }

    LaunchedEffect(Unit) {
        viewModel.loadCommands(serverId)
    }

    // Collect toast messages
    LaunchedEffect(Unit) {
        viewModel.toastMessage.collectLatest { message ->
            snackbarHostState.showSnackbar(message)
        }
    }

    LaunchedEffect(uiState) {
        val successState = uiState as? CommandListUiState.Success
        if (successState != null) {
            val remoteEnabled = successState.commands.isNotEmpty()
            if (!remoteEnabled && selectedTab == CommandListTab.Remote) {
                selectedTab = CommandListTab.List
            }
            if (!remoteEnabled && isEditingRemote) {
                isEditingRemote = false
            }
        } else if (selectedTab == CommandListTab.Remote) {
            selectedTab = CommandListTab.List
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
                        viewModel.loadCommands(serverId)
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
                            viewModel.loadCommands(serverId)
                        }
                    )
                }

                is CommandListUiState.Success -> {
                    if (state.commands.isEmpty()) {
                        EmptyCommandsView()
                    } else {
                        val availableTabs = listOf(CommandListTab.List, CommandListTab.Remote)

                        LaunchedEffect(state.remoteLayoutSpec, isEditingRemote) {
                            if (!isEditingRemote) {
                                layoutDraft = (state.remoteLayoutSpec ?: RemoteLayoutSpec.withDefaultSections())
                            }
                        }

                        CommandListTabRow(
                            tabs = availableTabs,
                            selectedTab = selectedTab,
                            onTabSelected = { selectedTab = it }
                        )
                        Spacer(modifier = Modifier.height(12.dp))

                        if (selectedTab == CommandListTab.Remote) {
                            RemoteTabContent(
                                state = state,
                                isEditing = isEditingRemote,
                                layoutDraft = layoutDraft,
                                onEditToggle = { editing ->
                                    if (editing) {
                                        layoutDraft = state.remoteLayoutSpec ?: RemoteLayoutSpec.withDefaultSections()
                                    }
                                    isEditingRemote = editing
                                },
                                onDraftChanged = { layoutDraft = it },
                                onSave = {
                                    viewModel.saveRemoteLayoutSpec(layoutDraft)
                                    isEditingRemote = false
                                },
                                onReset = {
                                    layoutDraft = RemoteLayoutSpec.withDefaultSections()
                                },
                                onQuickAction = { action ->
                                    val command = state.commands.find { it.id == action.id } ?: return@RemoteTabContent
                                    when {
                                        command.parameters.isNotEmpty() -> {
                                            onNavigateToCommandExecution(serverId, command.id)
                                        }
                                        action.requiresConfirmation -> {
                                            selectedCommand = command
                                            showConfirmationDialog = true
                                        }
                                        else -> {
                                            viewModel.triggerQuickCommand(
                                                command = command,
                                                showOutputOverride = action.showOutput
                                            )
                                            if (action.showOutput) {
                                                onNavigateToCommandExecution(serverId, command.id)
                                            }
                                        }
                                    }
                                },
                                modifier = Modifier.fillMaxSize()
                            )
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
                                            onNavigateToCommandExecution(serverId, command.id)
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
                    onNavigateToCommandExecution(serverId, cmd.id)
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

private enum class CommandListTab { List, Remote }

@Composable
private fun CommandListTabRow(
    tabs: List<CommandListTab>,
    selectedTab: CommandListTab,
    onTabSelected: (CommandListTab) -> Unit,
    modifier: Modifier = Modifier
) {
    val safeIndex = tabs.indexOf(selectedTab).takeIf { it >= 0 } ?: 0

    PrimaryTabRow(
        selectedTabIndex = safeIndex,
        modifier = modifier
    ) {
        tabs.forEachIndexed { index, tab ->
            val label = when (tab) {
                CommandListTab.List -> "List"
                CommandListTab.Remote -> "Remote"
            }

            Tab(
                selected = index == safeIndex,
                onClick = { onTabSelected(tab) },
                text = { Text(label) }
            )
        }
    }
}

@Composable
private fun RemoteTabContent(
    state: CommandListUiState.Success,
    isEditing: Boolean,
    layoutDraft: RemoteLayoutSpec,
    onEditToggle: (Boolean) -> Unit,
    onDraftChanged: (RemoteLayoutSpec) -> Unit,
    onSave: () -> Unit,
    onReset: () -> Unit,
    onQuickAction: (RemoteQuickAction) -> Unit,
    modifier: Modifier = Modifier
) {
    Box(modifier = modifier) {
        if (isEditing) {
            RemoteLayoutBuilderScreen(
                commands = state.commands,
                initialSpec = layoutDraft,
                onSpecChanged = onDraftChanged,
                modifier = Modifier.fillMaxSize()
            )

            BuilderActionBar(
                onCancel = { onEditToggle(false) },
                onReset = onReset,
                onSave = onSave,
                modifier = Modifier.align(Alignment.BottomCenter)
            )
        } else {
            if (state.remotePanel.hasContent) {
                val scrollState = rememberScrollState()
                RemotePanelScreen(
                    uiModel = state.remotePanel,
                    onQuickActionClick = onQuickAction,
                    modifier = Modifier
                        .fillMaxSize()
                        .verticalScroll(scrollState)
                )
            } else {
                RemotePanelPlaceholder(
                    modifier = Modifier.align(Alignment.Center),
                    onStartEdit = { onEditToggle(true) }
                )
            }

            FloatingActionButton(
                onClick = { onEditToggle(true) },
                modifier = Modifier
                    .align(Alignment.BottomEnd)
                    .padding(16.dp)
            ) {
                Icon(
                    imageVector = Icons.Default.Edit,
                    contentDescription = "Customize Remote"
                )
            }
        }
    }
}

@Composable
private fun BuilderActionBar(
    onCancel: () -> Unit,
    onReset: () -> Unit,
    onSave: () -> Unit,
    modifier: Modifier = Modifier
) {
    Row(
        modifier = modifier
            .fillMaxWidth()
            .background(MaterialTheme.colorScheme.surface.copy(alpha = 0.95f))
            .padding(16.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp)
    ) {
        TextButton(onClick = onCancel) {
            Text("Cancel")
        }
        OutlinedButton(onClick = onReset) {
            Text("Reset")
        }
        Spacer(modifier = Modifier.weight(1f))
        Button(onClick = onSave) {
            Icon(imageVector = Icons.Default.Save, contentDescription = null)
            Spacer(modifier = Modifier.width(8.dp))
            Text("Save")
        }
    }
}

@Composable
private fun RemotePanelPlaceholder(
    modifier: Modifier = Modifier,
    onStartEdit: () -> Unit
) {
    Column(
        modifier = modifier,
        verticalArrangement = Arrangement.Center,
        horizontalAlignment = Alignment.CenterHorizontally
    ) {
        Icon(
            imageVector = Icons.Default.Edit,
            contentDescription = null,
            tint = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.size(48.dp)
        )
        Spacer(modifier = Modifier.height(12.dp))
        Text(
            text = "No remote layout yet",
            style = MaterialTheme.typography.titleMedium
        )
        Spacer(modifier = Modifier.height(8.dp))
        Text(
            text = "Drag commands from the library to craft your control panel.",
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            textAlign = TextAlign.Center
        )
        Spacer(modifier = Modifier.height(16.dp))
        Button(onClick = onStartEdit) {
            Text("Start Customizing")
        }
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
            viewModel.triggerQuickCommand(command)
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

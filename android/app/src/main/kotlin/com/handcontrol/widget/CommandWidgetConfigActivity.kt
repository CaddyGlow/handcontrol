package com.handcontrol.widget

import android.app.Activity
import android.appwidget.AppWidgetManager
import android.content.Intent
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.Computer
import androidx.compose.material.icons.filled.PlayArrow
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.lifecycleScope
import com.handcontrol.data.commands.Command
import com.handcontrol.data.commands.CommandKind
import com.handcontrol.data.commands.CommandRepository
import com.handcontrol.data.commands.CommandSessionMode
import com.handcontrol.data.database.EnrolledServerEntity
import com.handcontrol.data.database.EnrolledServerRepository
import com.handcontrol.ui.theme.HandControlTheme
import dagger.hilt.android.AndroidEntryPoint
import kotlinx.coroutines.launch
import javax.inject.Inject

@AndroidEntryPoint
class CommandWidgetConfigActivity : ComponentActivity() {

    @Inject
    lateinit var enrolledServerRepository: EnrolledServerRepository

    @Inject
    lateinit var commandRepository: CommandRepository

    private var appWidgetId = AppWidgetManager.INVALID_APPWIDGET_ID

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)

        // Set result to CANCELED initially
        setResult(Activity.RESULT_CANCELED)

        // Get widget ID from intent
        appWidgetId = intent?.extras?.getInt(
            AppWidgetManager.EXTRA_APPWIDGET_ID,
            AppWidgetManager.INVALID_APPWIDGET_ID
        ) ?: AppWidgetManager.INVALID_APPWIDGET_ID

        if (appWidgetId == AppWidgetManager.INVALID_APPWIDGET_ID) {
            finish()
            return
        }

        setContent {
            HandControlTheme {
                WidgetConfigScreen(
                    enrolledServerRepository = enrolledServerRepository,
                    commandRepository = commandRepository,
                    onConfigComplete = { serverId, serverName, command ->
                        saveWidgetConfiguration(serverId, serverName, command)
                    },
                    onCancel = { finish() }
                )
            }
        }
    }

    private fun saveWidgetConfiguration(
        serverId: String,
        serverName: String,
        command: Command
    ) {
        // Save configuration
        CommandWidgetPreferences.saveWidgetConfig(
            this,
            appWidgetId,
            CommandWidgetPreferences.WidgetConfig(
                serverId = serverId,
                commandId = command.id,
                commandName = command.name,
                serverName = serverName,
                showServerName = false
            )
        )

        // Update widget
        val appWidgetManager = AppWidgetManager.getInstance(this)
        CommandWidgetProvider.updateAppWidget(this, appWidgetManager, appWidgetId)

        // Return success
        val resultValue = Intent().apply {
            putExtra(AppWidgetManager.EXTRA_APPWIDGET_ID, appWidgetId)
        }
        setResult(Activity.RESULT_OK, resultValue)
        finish()
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun WidgetConfigScreen(
    enrolledServerRepository: EnrolledServerRepository,
    commandRepository: CommandRepository,
    onConfigComplete: (String, String, Command) -> Unit,
    onCancel: () -> Unit
) {
    var configStep by remember { mutableStateOf(ConfigStep.SERVER_SELECTION) }
    var selectedServer by remember { mutableStateOf<EnrolledServerEntity?>(null) }
    var servers by remember { mutableStateOf<List<EnrolledServerEntity>>(emptyList()) }
    var commands by remember { mutableStateOf<List<Command>>(emptyList()) }
    var isLoadingServers by remember { mutableStateOf(true) }
    var isLoadingCommands by remember { mutableStateOf(false) }
    var errorMessage by remember { mutableStateOf<String?>(null) }
    val scope = rememberCoroutineScope()

    // Load servers
    LaunchedEffect(Unit) {
        scope.launch {
            try {
                enrolledServerRepository.allServers.collect { serverList ->
                    servers = serverList
                    isLoadingServers = false
                }
            } catch (e: Exception) {
                errorMessage = "Failed to load servers: ${e.message}"
                isLoadingServers = false
            }
        }
    }

    Scaffold(
        topBar = {
            TopAppBar(
                title = {
                    Text(
                        when (configStep) {
                            ConfigStep.SERVER_SELECTION -> "Select Server"
                            ConfigStep.COMMAND_SELECTION -> "Select Command"
                        }
                    )
                },
                navigationIcon = {
                    IconButton(onClick = {
                        when (configStep) {
                            ConfigStep.SERVER_SELECTION -> onCancel()
                            ConfigStep.COMMAND_SELECTION -> {
                                configStep = ConfigStep.SERVER_SELECTION
                                selectedServer = null
                                commands = emptyList()
                            }
                        }
                    }) {
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
            modifier = Modifier
                .fillMaxSize()
                .padding(paddingValues)
        ) {
            when (configStep) {
                ConfigStep.SERVER_SELECTION -> {
                    if (isLoadingServers) {
                        LoadingView()
                    } else if (errorMessage != null) {
                        ErrorView(errorMessage!!)
                    } else if (servers.isEmpty()) {
                        EmptyServersView()
                    } else {
                        ServerSelectionList(
                            servers = servers,
                            onServerSelected = { server ->
                                selectedServer = server
                                configStep = ConfigStep.COMMAND_SELECTION
                                isLoadingCommands = true
                                scope.launch {
                                    try {
                                        val result = commandRepository.listCommands(server.serverId)
                                        val loaded = result.getOrElse { emptyList() }
                                        commands = loaded.filter {
                                            it.sessionMode == CommandSessionMode.ONE_SHOT &&
                                                it.kind == CommandKind.SHELL_SCRIPT
                                        }
                                        isLoadingCommands = false
                                    } catch (e: Exception) {
                                        errorMessage = "Failed to load commands: ${e.message}"
                                        isLoadingCommands = false
                                    }
                                }
                            }
                        )
                    }
                }
                ConfigStep.COMMAND_SELECTION -> {
                    if (isLoadingCommands) {
                        LoadingView()
                    } else if (errorMessage != null) {
                        ErrorView(errorMessage!!)
                    } else if (commands.isEmpty()) {
                        EmptyCommandsView()
                    } else {
                        CommandSelectionList(
                            commands = commands,
                            onCommandSelected = { command ->
                                selectedServer?.let { server ->
                                    onConfigComplete(server.serverId, server.serverName, command)
                                }
                            }
                        )
                    }
                }
            }
        }
    }
}

private enum class ConfigStep {
    SERVER_SELECTION,
    COMMAND_SELECTION
}

@Composable
private fun ServerSelectionList(
    servers: List<EnrolledServerEntity>,
    onServerSelected: (EnrolledServerEntity) -> Unit
) {
    LazyColumn(
        modifier = Modifier.fillMaxSize(),
        contentPadding = androidx.compose.foundation.layout.PaddingValues(16.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp)
    ) {
        items(servers) { server ->
            Card(
                modifier = Modifier
                    .fillMaxWidth()
                    .clickable { onServerSelected(server) },
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
                        imageVector = Icons.Default.Computer,
                        contentDescription = null,
                        tint = MaterialTheme.colorScheme.primary,
                        modifier = Modifier.size(32.dp)
                    )
                    Spacer(modifier = Modifier.size(16.dp))
                    Column(modifier = Modifier.weight(1f)) {
                        Text(
                            text = server.serverName,
                            style = MaterialTheme.typography.titleMedium
                        )
                        Text(
                            text = server.ips.firstOrNull() ?: "Unknown",
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant
                        )
                    }
                }
            }
        }
    }
}

@Composable
private fun CommandSelectionList(
    commands: List<Command>,
    onCommandSelected: (Command) -> Unit
) {
    LazyColumn(
        modifier = Modifier.fillMaxSize(),
        contentPadding = androidx.compose.foundation.layout.PaddingValues(16.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp)
    ) {
        items(commands) { command ->
            Card(
                modifier = Modifier
                    .fillMaxWidth()
                    .clickable { onCommandSelected(command) },
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
                    Spacer(modifier = Modifier.size(16.dp))
                    Column(modifier = Modifier.weight(1f)) {
                        Text(
                            text = command.name,
                            style = MaterialTheme.typography.titleMedium
                        )
                        Text(
                            text = command.description,
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            maxLines = 2,
                            overflow = TextOverflow.Ellipsis
                        )
                        if (command.parameters.isNotEmpty()) {
                            Spacer(modifier = Modifier.height(4.dp))
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
    }
}

@Composable
private fun LoadingView() {
    Column(
        modifier = Modifier.fillMaxSize(),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.Center
    ) {
        CircularProgressIndicator()
        Spacer(modifier = Modifier.height(16.dp))
        Text("Loading...")
    }
}

@Composable
private fun ErrorView(message: String) {
    Column(
        modifier = Modifier
            .fillMaxSize()
            .padding(16.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.Center
    ) {
        Text(
            text = message,
            style = MaterialTheme.typography.bodyLarge,
            color = MaterialTheme.colorScheme.error
        )
    }
}

@Composable
private fun EmptyServersView() {
    Column(
        modifier = Modifier
            .fillMaxSize()
            .padding(16.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.Center
    ) {
        Text(
            text = "No servers enrolled",
            style = MaterialTheme.typography.titleMedium
        )
        Spacer(modifier = Modifier.height(8.dp))
        Text(
            text = "Please enroll a server first",
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant
        )
    }
}

@Composable
private fun EmptyCommandsView() {
    Column(
        modifier = Modifier
            .fillMaxSize()
            .padding(16.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.Center
    ) {
        Text(
            text = "No commands available",
            style = MaterialTheme.typography.titleMedium
        )
        Spacer(modifier = Modifier.height(8.dp))
        Text(
            text = "This server has no commands configured",
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant
        )
    }
}

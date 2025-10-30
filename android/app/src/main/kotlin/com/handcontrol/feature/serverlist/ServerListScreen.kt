package com.handcontrol.feature.serverlist

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
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.CheckCircle
import androidx.compose.material.icons.filled.Computer
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.Info
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FloatingActionButton
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
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
import com.handcontrol.data.database.EnrolledServerEntity
import com.handcontrol.ui.theme.HandControlTheme
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ServerListScreen(
    onNavigateBack: () -> Unit,
    onNavigateToServerDiscovery: () -> Unit,
    onNavigateToServer: (String, Int) -> Unit,
    onNavigateToServerDetails: (String) -> Unit,
    modifier: Modifier = Modifier,
    viewModel: ServerListViewModel = hiltViewModel()
) {
    val uiState by viewModel.uiState.collectAsStateWithLifecycle()
    var serverToDelete by remember { mutableStateOf<EnrolledServerEntity?>(null) }

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("Enrolled Servers") },
                navigationIcon = {
                    IconButton(onClick = onNavigateBack) {
                        Icon(
                            imageVector = Icons.AutoMirrored.Filled.ArrowBack,
                            contentDescription = "Back"
                        )
                    }
                }
            )
        },
        floatingActionButton = {
            FloatingActionButton(
                onClick = onNavigateToServerDiscovery
            ) {
                Icon(
                    imageVector = Icons.Filled.Add,
                    contentDescription = "Add Server"
                )
            }
        },
        modifier = modifier
    ) { paddingValues ->
        when (val state = uiState) {
            is ServerListUiState.Loading -> {
                Column(
                    modifier = Modifier
                        .fillMaxSize()
                        .padding(paddingValues),
                    horizontalAlignment = Alignment.CenterHorizontally,
                    verticalArrangement = Arrangement.Center
                ) {
                    CircularProgressIndicator(
                        modifier = Modifier.size(48.dp),
                        color = MaterialTheme.colorScheme.primary
                    )
                }
            }

            is ServerListUiState.Empty -> {
                EmptyServerListView(
                    onAddServer = onNavigateToServerDiscovery,
                    modifier = Modifier
                        .fillMaxSize()
                        .padding(paddingValues)
                )
            }

            is ServerListUiState.Success -> {
                LazyColumn(
                    modifier = Modifier
                        .fillMaxSize()
                        .padding(paddingValues),
                    contentPadding = PaddingValues(16.dp),
                    verticalArrangement = Arrangement.spacedBy(12.dp)
                ) {
                    items(
                        items = state.servers,
                        key = { it.serverId }
                    ) { server ->
                        ServerCard(
                            server = server,
                            onClick = {
                                viewModel.connectToServer(server.serverId)
                                // Use first IP from the list or fallback to deprecated serverHost
                                val host = server.ips.firstOrNull() ?: server.serverHost ?: ""
                                onNavigateToServer(host, server.serverPort)
                            },
                            onDelete = {
                                serverToDelete = server
                            },
                            onViewDetails = {
                                onNavigateToServerDetails(server.serverId)
                            }
                        )
                    }
                }
            }
        }

        // Delete confirmation dialog
        serverToDelete?.let { server ->
            AlertDialog(
                onDismissRequest = { serverToDelete = null },
                title = { Text("Remove Server") },
                text = {
                    Text("Are you sure you want to remove ${server.serverName}? This will delete all enrollment data for this server.")
                },
                confirmButton = {
                    TextButton(
                        onClick = {
                            viewModel.deleteServer(server.serverId)
                            serverToDelete = null
                        }
                    ) {
                        Text("Remove")
                    }
                },
                dismissButton = {
                    TextButton(onClick = { serverToDelete = null }) {
                        Text("Cancel")
                    }
                }
            )
        }
    }
}

@Composable
private fun ServerCard(
    server: EnrolledServerEntity,
    onClick: () -> Unit,
    onDelete: () -> Unit,
    onViewDetails: () -> Unit,
    modifier: Modifier = Modifier
) {
    Card(
        modifier = modifier
            .fillMaxWidth()
            .clickable(onClick = onClick),
        elevation = CardDefaults.cardElevation(defaultElevation = 2.dp)
    ) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(16.dp),
            verticalAlignment = Alignment.CenterVertically
        ) {
            Icon(
                imageVector = Icons.Filled.Computer,
                contentDescription = null,
                tint = MaterialTheme.colorScheme.primary,
                modifier = Modifier.size(48.dp)
            )

            Column(
                modifier = Modifier
                    .weight(1f)
                    .padding(horizontal = 16.dp)
            ) {
                Row(
                    verticalAlignment = Alignment.CenterVertically
                ) {
                    Text(
                        text = server.serverName,
                        style = MaterialTheme.typography.titleMedium,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis
                    )

                    if (server.lastConnected != null) {
                        Icon(
                            imageVector = Icons.Filled.CheckCircle,
                            contentDescription = "Last connected",
                            tint = MaterialTheme.colorScheme.primary,
                            modifier = Modifier
                                .padding(start = 8.dp)
                                .size(16.dp)
                        )
                    }
                }

                Spacer(modifier = Modifier.height(4.dp))

                Text(
                    text = "${server.ips.firstOrNull() ?: server.serverHost ?: "unknown"}:${server.serverPort}",
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )

                Spacer(modifier = Modifier.height(4.dp))

                val dateFormat = remember { SimpleDateFormat("MMM d, yyyy h:mm a", Locale.getDefault()) }
                val lastConnectedText = if (server.lastConnected != null) {
                    "Last used: ${dateFormat.format(Date(server.lastConnected))}"
                } else {
                    "Enrolled: ${dateFormat.format(Date(server.enrolledAt))}"
                }

                Text(
                    text = lastConnectedText,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )
            }

            IconButton(onClick = onViewDetails) {
                Icon(
                    imageVector = Icons.Filled.Info,
                    contentDescription = "View server details",
                    tint = MaterialTheme.colorScheme.primary
                )
            }

            IconButton(onClick = onDelete) {
                Icon(
                    imageVector = Icons.Filled.Delete,
                    contentDescription = "Delete server",
                    tint = MaterialTheme.colorScheme.error
                )
            }
        }
    }
}

@Composable
private fun EmptyServerListView(
    onAddServer: () -> Unit,
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
            imageVector = Icons.Filled.Computer,
            contentDescription = null,
            tint = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.size(80.dp)
        )

        Spacer(modifier = Modifier.height(24.dp))

        Text(
            text = "No Servers Enrolled",
            style = MaterialTheme.typography.headlineMedium,
            textAlign = TextAlign.Center
        )

        Spacer(modifier = Modifier.height(16.dp))

        Text(
            text = "Add a server to start controlling your computer remotely",
            style = MaterialTheme.typography.bodyLarge,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            textAlign = TextAlign.Center
        )

        Spacer(modifier = Modifier.height(48.dp))

        FloatingActionButton(onClick = onAddServer) {
            Icon(
                imageVector = Icons.Filled.Add,
                contentDescription = "Add Server"
            )
        }
    }
}

@Preview(showBackground = true, showSystemUi = true)
@Composable
private fun ServerListPreview() {
    HandControlTheme {
        ServerListScreen(
            onNavigateBack = {},
            onNavigateToServerDiscovery = {},
            onNavigateToServer = { _, _ -> },
            onNavigateToServerDetails = {}
        )
    }
}

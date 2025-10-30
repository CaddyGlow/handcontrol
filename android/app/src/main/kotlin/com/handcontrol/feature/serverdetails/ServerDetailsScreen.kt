package com.handcontrol.feature.serverdetails

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.CheckCircle
import androidx.compose.material.icons.filled.ContentCopy
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.Error
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
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
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.hilt.navigation.compose.hiltViewModel
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.handcontrol.core.model.ServerDetailInfo
import com.handcontrol.core.model.ServerHealthStatus
import com.handcontrol.data.database.ConnectionMode
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import java.util.concurrent.TimeUnit

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ServerDetailsScreen(
    onNavigateBack: () -> Unit,
    modifier: Modifier = Modifier,
    viewModel: ServerDetailsViewModel = hiltViewModel()
) {
    val uiState by viewModel.uiState.collectAsStateWithLifecycle()
    var showDeleteDialog by remember { mutableStateOf(false) }

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("Server Details") },
                navigationIcon = {
                    IconButton(onClick = onNavigateBack) {
                        Icon(
                            imageVector = Icons.AutoMirrored.Filled.ArrowBack,
                            contentDescription = "Back"
                        )
                    }
                },
                actions = {
                    IconButton(onClick = { viewModel.checkHealth() }) {
                        Icon(
                            imageVector = Icons.Default.Refresh,
                            contentDescription = "Refresh health check"
                        )
                    }
                    IconButton(onClick = { showDeleteDialog = true }) {
                        Icon(
                            imageVector = Icons.Default.Delete,
                            contentDescription = "Delete server"
                        )
                    }
                }
            )
        },
        modifier = modifier
    ) { paddingValues ->
        when (val state = uiState) {
            is ServerDetailsUiState.Loading -> {
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

            is ServerDetailsUiState.Error -> {
                Column(
                    modifier = Modifier
                        .fillMaxSize()
                        .padding(paddingValues)
                        .padding(16.dp),
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
                        text = state.message,
                        style = MaterialTheme.typography.bodyLarge,
                        textAlign = TextAlign.Center
                    )
                }
            }

            is ServerDetailsUiState.Success -> {
                ServerDetailsContent(
                    serverInfo = state.serverInfo,
                    onTestConnection = { viewModel.checkHealth() },
                    modifier = Modifier
                        .fillMaxSize()
                        .padding(paddingValues)
                )
            }
        }

        // Delete confirmation dialog
        if (showDeleteDialog) {
            AlertDialog(
                onDismissRequest = { showDeleteDialog = false },
                title = { Text("Remove Server") },
                text = {
                    Text("Are you sure you want to remove this server? This will delete all enrollment data.")
                },
                confirmButton = {
                    TextButton(
                        onClick = {
                            viewModel.deleteServer()
                            showDeleteDialog = false
                            onNavigateBack()
                        }
                    ) {
                        Text("Remove")
                    }
                },
                dismissButton = {
                    TextButton(onClick = { showDeleteDialog = false }) {
                        Text("Cancel")
                    }
                }
            )
        }
    }
}

@Composable
private fun ServerDetailsContent(
    serverInfo: ServerDetailInfo,
    onTestConnection: () -> Unit,
    modifier: Modifier = Modifier
) {
    Column(
        modifier = modifier
            .verticalScroll(rememberScrollState())
            .padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp)
    ) {
        // Header with server name and status
        ServerHeaderCard(serverInfo)

        // Connection Details Section
        InfoSection(title = "Connection Details") {
            ConnectionDetailsContent(serverInfo)
        }

        // Server Identity Section
        InfoSection(title = "Server Identity") {
            ServerIdentityContent(serverInfo)
        }

        // Timestamps Section
        InfoSection(title = "Activity") {
            TimestampsContent(serverInfo)
        }

        // Network Status Section
        InfoSection(title = "Network Status") {
            NetworkStatusContent(serverInfo, onTestConnection)
        }
    }
}

@Composable
private fun ServerHeaderCard(serverInfo: ServerDetailInfo) {
    Card(
        modifier = Modifier.fillMaxWidth(),
        colors = CardDefaults.cardColors(
            containerColor = MaterialTheme.colorScheme.primaryContainer
        )
    ) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(16.dp),
            verticalAlignment = Alignment.CenterVertically
        ) {
            Column(modifier = Modifier.weight(1f)) {
                Text(
                    text = serverInfo.serverName,
                    style = MaterialTheme.typography.headlineSmall,
                    fontWeight = FontWeight.Bold
                )
                Text(
                    text = serverInfo.serverId,
                    style = MaterialTheme.typography.bodySmall,
                    fontFamily = FontFamily.Monospace,
                    color = MaterialTheme.colorScheme.onPrimaryContainer.copy(alpha = 0.7f)
                )
            }
            Spacer(modifier = Modifier.width(8.dp))
            HealthStatusIndicator(serverInfo.healthStatus)
        }
    }
}

@Composable
private fun HealthStatusIndicator(healthStatus: ServerHealthStatus) {
    val (icon, tint, label) = when (healthStatus) {
        is ServerHealthStatus.Connected -> Triple(
            Icons.Default.CheckCircle,
            Color(0xFF4CAF50),
            "${healthStatus.latencyMs}ms"
        )
        is ServerHealthStatus.Disconnected -> Triple(
            Icons.Default.Error,
            MaterialTheme.colorScheme.error,
            "Offline"
        )
        is ServerHealthStatus.Checking -> Triple(
            null,
            MaterialTheme.colorScheme.primary,
            "Checking..."
        )
        is ServerHealthStatus.Unknown -> Triple(
            Icons.Default.Error,
            MaterialTheme.colorScheme.outline,
            "Unknown"
        )
    }

    Column(horizontalAlignment = Alignment.CenterHorizontally) {
        if (icon != null) {
            Icon(
                imageVector = icon,
                contentDescription = null,
                tint = tint,
                modifier = Modifier.size(32.dp)
            )
        } else {
            CircularProgressIndicator(
                modifier = Modifier.size(32.dp),
                color = tint,
                strokeWidth = 3.dp
            )
        }
        Text(
            text = label,
            style = MaterialTheme.typography.labelSmall,
            color = tint
        )
    }
}

@Composable
private fun InfoSection(
    title: String,
    content: @Composable () -> Unit
) {
    Card(
        modifier = Modifier.fillMaxWidth(),
        elevation = CardDefaults.cardElevation(defaultElevation = 2.dp)
    ) {
        Column(modifier = Modifier.padding(16.dp)) {
            Text(
                text = title,
                style = MaterialTheme.typography.titleMedium,
                fontWeight = FontWeight.Bold,
                color = MaterialTheme.colorScheme.primary
            )
            Spacer(modifier = Modifier.height(12.dp))
            content()
        }
    }
}

@Composable
private fun ConnectionDetailsContent(serverInfo: ServerDetailInfo) {
    val context = LocalContext.current

    // Primary IP:Port
    InfoRow(
        label = "Primary Address",
        value = "${serverInfo.ips.firstOrNull() ?: "N/A"}:${serverInfo.serverPort}",
        isCopyable = true,
        context = context
    )

    if (serverInfo.ips.size > 1) {
        HorizontalDivider(modifier = Modifier.padding(vertical = 8.dp))
        Text(
            text = "All IP Addresses",
            style = MaterialTheme.typography.labelMedium,
            color = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.6f)
        )
        Spacer(modifier = Modifier.height(4.dp))
        serverInfo.ips.forEachIndexed { index, ip ->
            InfoRow(
                label = "IP ${index + 1}",
                value = ip,
                isCopyable = true,
                context = context
            )
        }
    }

    HorizontalDivider(modifier = Modifier.padding(vertical = 8.dp))
    InfoRow(
        label = "Connection Mode",
        value = when (serverInfo.lastConnectionMode) {
            ConnectionMode.DIRECT -> "Direct"
            ConnectionMode.RELAY -> "Relay"
            ConnectionMode.UNKNOWN -> "Unknown"
        }
    )
}

@Composable
private fun ServerIdentityContent(serverInfo: ServerDetailInfo) {
    val context = LocalContext.current

    InfoRow(
        label = "Server ID",
        value = serverInfo.serverId,
        isCopyable = true,
        isMonospace = true,
        context = context
    )
    HorizontalDivider(modifier = Modifier.padding(vertical = 8.dp))
    InfoRow(
        label = "mDNS Service Name",
        value = serverInfo.mdnsServiceName ?: "Not discovered"
    )
    HorizontalDivider(modifier = Modifier.padding(vertical = 8.dp))
    InfoRow(
        label = "Certificate Fingerprint",
        value = serverInfo.certFingerprint,
        isCopyable = true,
        isMonospace = true,
        context = context
    )
    HorizontalDivider(modifier = Modifier.padding(vertical = 8.dp))
    InfoRow(
        label = "Client ID",
        value = serverInfo.clientId,
        isCopyable = true,
        isMonospace = true,
        context = context
    )
}

@Composable
private fun TimestampsContent(serverInfo: ServerDetailInfo) {
    InfoRow(
        label = "Enrolled At",
        value = formatTimestamp(serverInfo.enrolledAt)
    )
    HorizontalDivider(modifier = Modifier.padding(vertical = 8.dp))
    InfoRow(
        label = "Last Connected",
        value = serverInfo.lastConnected?.let { formatRelativeTime(it) } ?: "Never"
    )
    if (serverInfo.isDiscoveredViaMdns && serverInfo.mdnsLastSeen != null) {
        HorizontalDivider(modifier = Modifier.padding(vertical = 8.dp))
        InfoRow(
            label = "Last Seen (mDNS)",
            value = formatRelativeTime(serverInfo.mdnsLastSeen)
        )
    }
}

@Composable
private fun NetworkStatusContent(
    serverInfo: ServerDetailInfo,
    onTestConnection: () -> Unit
) {
    InfoRow(
        label = "mDNS Discovery",
        value = if (serverInfo.isDiscoveredViaMdns) "Discoverable" else "Not found"
    )
    HorizontalDivider(modifier = Modifier.padding(vertical = 8.dp))
    InfoRow(
        label = "Health Status",
        value = when (val status = serverInfo.healthStatus) {
            is ServerHealthStatus.Connected -> "Connected (${status.latencyMs}ms)"
            is ServerHealthStatus.Disconnected -> "Unreachable${status.reason?.let { ": $it" } ?: ""}"
            is ServerHealthStatus.Checking -> "Checking..."
            is ServerHealthStatus.Unknown -> "Unknown"
        }
    )

    Spacer(modifier = Modifier.height(12.dp))
    OutlinedButton(
        onClick = onTestConnection,
        modifier = Modifier.fillMaxWidth()
    ) {
        Icon(
            imageVector = Icons.Default.Refresh,
            contentDescription = null,
            modifier = Modifier.size(18.dp)
        )
        Spacer(modifier = Modifier.width(8.dp))
        Text("Test Connection")
    }
}

@Composable
private fun InfoRow(
    label: String,
    value: String,
    isCopyable: Boolean = false,
    isMonospace: Boolean = false,
    context: Context? = null,
    modifier: Modifier = Modifier
) {
    Row(
        modifier = modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.SpaceBetween,
        verticalAlignment = Alignment.CenterVertically
    ) {
        Text(
            text = label,
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.6f),
            modifier = Modifier.weight(0.4f)
        )
        Row(
            modifier = Modifier.weight(0.6f),
            horizontalArrangement = Arrangement.End,
            verticalAlignment = Alignment.CenterVertically
        ) {
            Text(
                text = value,
                style = MaterialTheme.typography.bodyMedium,
                fontFamily = if (isMonospace) FontFamily.Monospace else FontFamily.Default,
                textAlign = TextAlign.End,
                modifier = Modifier.weight(1f, fill = false)
            )
            if (isCopyable && context != null) {
                Spacer(modifier = Modifier.width(4.dp))
                IconButton(
                    onClick = {
                        val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
                        val clip = ClipData.newPlainText(label, value)
                        clipboard.setPrimaryClip(clip)
                    },
                    modifier = Modifier.size(24.dp)
                ) {
                    Icon(
                        imageVector = Icons.Default.ContentCopy,
                        contentDescription = "Copy $label",
                        tint = MaterialTheme.colorScheme.primary,
                        modifier = Modifier.size(16.dp)
                    )
                }
            }
        }
    }
}

private fun formatTimestamp(timestamp: Long): String {
    val sdf = SimpleDateFormat("MMM dd, yyyy HH:mm", Locale.getDefault())
    return sdf.format(Date(timestamp))
}

private fun formatRelativeTime(timestamp: Long): String {
    val now = System.currentTimeMillis()
    val diff = now - timestamp

    return when {
        diff < TimeUnit.MINUTES.toMillis(1) -> "Just now"
        diff < TimeUnit.HOURS.toMillis(1) -> {
            val minutes = TimeUnit.MILLISECONDS.toMinutes(diff)
            "$minutes minute${if (minutes == 1L) "" else "s"} ago"
        }
        diff < TimeUnit.DAYS.toMillis(1) -> {
            val hours = TimeUnit.MILLISECONDS.toHours(diff)
            "$hours hour${if (hours == 1L) "" else "s"} ago"
        }
        diff < TimeUnit.DAYS.toMillis(7) -> {
            val days = TimeUnit.MILLISECONDS.toDays(diff)
            "$days day${if (days == 1L) "" else "s"} ago"
        }
        else -> formatTimestamp(timestamp)
    }
}

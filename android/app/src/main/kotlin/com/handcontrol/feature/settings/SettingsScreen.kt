package com.handcontrol.feature.settings

import android.content.Intent
import android.net.Uri
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.core.content.FileProvider
import androidx.hilt.navigation.compose.hiltViewModel
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.handcontrol.BuildConfig
import com.handcontrol.data.settings.*
import com.handcontrol.feature.settings.components.*

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SettingsScreen(
    onNavigateBack: () -> Unit,
    modifier: Modifier = Modifier,
    viewModel: SettingsViewModel = hiltViewModel()
) {
    val settings by viewModel.settings.collectAsStateWithLifecycle()
    val context = LocalContext.current
    var showClearDataDialog by remember { mutableStateOf(false) }
    var snackbarMessage by remember { mutableStateOf<String?>(null) }
    val snackbarHostState = remember { SnackbarHostState() }

    LaunchedEffect(snackbarMessage) {
        snackbarMessage?.let {
            snackbarHostState.showSnackbar(it)
            snackbarMessage = null
        }
    }

    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text("Settings") },
                navigationIcon = {
                    IconButton(onClick = onNavigateBack) {
                        Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
                    }
                }
            )
        },
        snackbarHost = { SnackbarHost(snackbarHostState) }
    ) { paddingValues ->
        LazyColumn(
            modifier = modifier
                .fillMaxSize()
                .padding(paddingValues),
            contentPadding = PaddingValues(16.dp),
            verticalArrangement = Arrangement.spacedBy(16.dp)
        ) {
            // Connection & Relay Section
            item {
                SettingsSection(title = "Connection & Relay") {
                    SwitchPreference(
                        title = "Enable Relay Fallback",
                        description = "Use relay server when direct connection fails",
                        checked = settings.enableRelayFallback,
                        onCheckedChange = viewModel::updateRelayFallback
                    )
                    DropdownPreference(
                        title = "IPv6 Preference",
                        description = "Network protocol preference",
                        selectedOption = settings.ipv6Preference,
                        options = IpPreference.entries.toList(),
                        onOptionSelected = viewModel::updateIpv6Preference,
                        optionLabel = { it.toDisplayString() }
                    )
                    SliderPreference(
                        title = "Direct Connection Timeout",
                        description = "${settings.directConnectionTimeoutSeconds}s per IP address",
                        value = settings.directConnectionTimeoutSeconds.toFloat(),
                        valueRange = 1f..30f,
                        steps = 28,
                        onValueChange = { viewModel.updateDirectConnectionTimeout(it.toInt()) }
                    )
                    SwitchPreference(
                        title = "Auto-Discovery",
                        description = "Automatically discover servers via mDNS",
                        checked = settings.autoDiscovery,
                        onCheckedChange = viewModel::updateAutoDiscovery
                    )
                    if (settings.autoDiscovery) {
                        SliderPreference(
                            title = "Discovery Timeout",
                            description = "${settings.discoveryTimeoutSeconds}s mDNS search duration",
                            value = settings.discoveryTimeoutSeconds.toFloat(),
                            valueRange = 1f..30f,
                            steps = 28,
                            onValueChange = { viewModel.updateDiscoveryTimeout(it.toInt()) }
                        )
                    }
                }
            }

            // Appearance Section
            item {
                SettingsSection(title = "Appearance") {
                    DropdownPreference(
                        title = "Theme",
                        description = "App theme preference",
                        selectedOption = settings.theme,
                        options = Theme.entries.toList(),
                        onOptionSelected = viewModel::updateTheme,
                        optionLabel = { it.toDisplayString() }
                    )
                }
            }

            // Advanced Section (Developer/Debug Settings)
            item {
                ExpandableSettingsSection(
                    title = "Advanced",
                    initiallyExpanded = false
                ) {
                    DropdownPreference(
                        title = "Log Level",
                        description = "Logging verbosity",
                        selectedOption = settings.logLevel,
                        options = LogLevel.entries.toList(),
                        onOptionSelected = viewModel::updateLogLevel,
                        optionLabel = { it.name }
                    )
                    SwitchPreference(
                        title = "Enable Network Logging",
                        description = "Log gRPC requests/responses",
                        checked = settings.enableNetworkLogging,
                        onCheckedChange = viewModel::updateNetworkLogging,
                        warning = if (settings.enableNetworkLogging) "May log sensitive data" else null
                    )
                    SwitchPreference(
                        title = "Enable Network Diagnostics",
                        description = "Show connection diagnostics in UI",
                        checked = settings.enableNetworkDiagnostics,
                        onCheckedChange = viewModel::updateNetworkDiagnostics
                    )
                    HorizontalDivider(modifier = Modifier.padding(vertical = 8.dp))
                    ButtonPreference(
                        title = "Export Logs",
                        description = "Export application logs for debugging",
                        buttonText = "Export",
                        onClick = {
                            viewModel.exportLogs(context) { result ->
                                result
                                    .onSuccess { file ->
                                        if (file != null) {
                                            val uri = FileProvider.getUriForFile(
                                                context,
                                                "${context.packageName}.fileprovider",
                                                file
                                            )
                                            val shareIntent = Intent(Intent.ACTION_SEND).apply {
                                                type = "text/plain"
                                                putExtra(Intent.EXTRA_STREAM, uri)
                                                addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
                                            }
                                            context.startActivity(Intent.createChooser(shareIntent, "Export Logs"))
                                            snackbarMessage = "Logs exported successfully"
                                        } else {
                                            snackbarMessage = "No logs available to export"
                                        }
                                    }
                                    .onFailure {
                                        snackbarMessage = "Failed to export logs"
                                    }
                            }
                        }
                    )
                    ButtonPreference(
                        title = "Clear All Data",
                        description = "Remove all servers and reset settings",
                        buttonText = "Clear",
                        destructive = true,
                        onClick = { showClearDataDialog = true }
                    )
                }
            }

            // About Section
            item {
                SettingsSection(title = "About") {
                    InfoPreference(
                        title = "Version",
                        value = BuildConfig.VERSION_NAME
                    )
                    InfoPreference(
                        title = "Build Number",
                        value = BuildConfig.VERSION_CODE.toString()
                    )
                    ClickablePreference(
                        title = "Open Source Licenses",
                        onClick = {
                            val licenseUrl = "https://github.com/hand-engineering/handcontrol/blob/main/LICENSE"
                            val intent = Intent(Intent.ACTION_VIEW, Uri.parse(licenseUrl))
                            if (intent.resolveActivity(context.packageManager) != null) {
                                context.startActivity(intent)
                            } else {
                                snackbarMessage = "No application available to view licenses"
                            }
                        }
                    )
                }
            }
        }

        if (showClearDataDialog) {
            ClearDataConfirmationDialog(
                onConfirm = {
                    viewModel.clearAllData { result ->
                        showClearDataDialog = false
                        if (result.isSuccess) {
                            onNavigateBack()
                        }
                    }
                },
                onDismiss = { showClearDataDialog = false }
            )
        }
    }
}

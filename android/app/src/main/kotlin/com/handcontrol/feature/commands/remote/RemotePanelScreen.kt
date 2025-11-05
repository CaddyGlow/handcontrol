package com.handcontrol.feature.commands.remote

import androidx.compose.foundation.background
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Bolt
import androidx.compose.material.icons.filled.Speed
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.surfaceColorAtElevation
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.tooling.preview.Preview
import androidx.compose.ui.unit.dp
import com.handcontrol.ui.theme.HandControlTheme

@Composable
fun RemotePanelScreen(
    uiModel: RemotePanelUiModel,
    modifier: Modifier = Modifier,
    feedbackCommandId: String? = null,
    onDismissFeedback: () -> Unit = {},
    onQuickActionClick: (RemoteQuickAction) -> Unit = {},
    onAdjustmentChange: (RemoteAdjustment) -> Unit = {},
    onTelemetryClick: (RemoteTelemetryCard) -> Unit = {}
) {
    Column(
        modifier = modifier
            .padding(vertical = 16.dp)
            .padding(horizontal = 16.dp),
        verticalArrangement = Arrangement.spacedBy(24.dp)
    ) {
        if (uiModel.quickActions.isNotEmpty()) {
            QuickActionsSection(
                actions = uiModel.quickActions,
                feedbackCommandId = feedbackCommandId,
                onDismissFeedback = onDismissFeedback,
                onQuickActionClick = onQuickActionClick
            )
        }

        if (uiModel.adjustments.isNotEmpty()) {
            AdjustmentsSection(
                groups = uiModel.adjustments,
                onAdjustmentSelected = onAdjustmentChange
            )
        }

        if (uiModel.telemetry.isNotEmpty()) {
            TelemetrySection(
                cards = uiModel.telemetry,
                onTelemetryClick = onTelemetryClick
            )
        }
    }
}

@Composable
private fun QuickActionsSection(
    actions: List<RemoteQuickAction>,
    feedbackCommandId: String?,
    onDismissFeedback: () -> Unit,
    onQuickActionClick: (RemoteQuickAction) -> Unit,
    modifier: Modifier = Modifier
) {
    Column(modifier = modifier.fillMaxWidth()) {
        Text(
            text = "Quick Actions",
            style = MaterialTheme.typography.titleMedium
        )
        Spacer(modifier = Modifier.height(12.dp))

        actions.chunked(2).forEach { rowActions ->
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(12.dp)
            ) {
                rowActions.forEach { action ->
                    RemoteQuickActionCard(
                        action = action,
                        showFeedback = feedbackCommandId == action.id,
                        onDismissFeedback = onDismissFeedback,
                        onClick = onQuickActionClick,
                        modifier = Modifier
                            .weight(1f)
                    )
                }
                if (rowActions.size == 1) {
                    Spacer(modifier = Modifier.weight(1f))
                }
            }
            Spacer(modifier = Modifier.height(12.dp))
        }
    }
}

@Composable
private fun RemoteQuickActionCard(
    action: RemoteQuickAction,
    showFeedback: Boolean,
    onDismissFeedback: () -> Unit,
    onClick: (RemoteQuickAction) -> Unit,
    modifier: Modifier = Modifier
) {
    val alpha = if (action.isEnabled) 1f else 0.4f
    Box(modifier = modifier) {
        Card(
            modifier = Modifier.fillMaxWidth(),
            enabled = action.isEnabled,
            onClick = { onClick(action) },
            colors = CardDefaults.cardColors(
                containerColor = MaterialTheme.colorScheme.surfaceVariant
            )
        ) {
            Column(
                modifier = Modifier
                    .padding(20.dp)
                    .alpha(alpha),
                verticalArrangement = Arrangement.spacedBy(12.dp)
            ) {
                Icon(
                    imageVector = Icons.Default.Bolt,
                    contentDescription = null,
                    tint = MaterialTheme.colorScheme.primary,
                    modifier = Modifier.size(24.dp)
                )
                Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                    Text(
                        text = action.title,
                        style = MaterialTheme.typography.titleMedium,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis
                    )
                    action.subtitle?.let {
                        Text(
                            text = it,
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            maxLines = 2,
                            overflow = TextOverflow.Ellipsis
                        )
                    }
                }
            }
        }

        // Visual feedback overlay
        if (showFeedback) {
            com.handcontrol.ui.components.CommandExecutionFeedback(
                visible = true,
                onDismiss = onDismissFeedback,
                modifier = Modifier.matchParentSize()
            )
        }
    }
}

@Composable
private fun AdjustmentsSection(
    groups: List<RemoteAdjustmentGroup>,
    onAdjustmentSelected: (RemoteAdjustment) -> Unit,
    modifier: Modifier = Modifier
) {
    Column(modifier = modifier.fillMaxWidth()) {
        Text(
            text = "Adjustments",
            style = MaterialTheme.typography.titleMedium
        )
        Spacer(modifier = Modifier.height(12.dp))

        val scrollState = rememberScrollState()
        Row(
            modifier = Modifier
                .horizontalScroll(scrollState)
                .padding(horizontal = 4.dp),
            horizontalArrangement = Arrangement.spacedBy(16.dp)
        ) {
            groups.forEach { group ->
                Card(
                    modifier = Modifier.width(240.dp),
                    colors = CardDefaults.cardColors(
                        containerColor = MaterialTheme.colorScheme.surface
                    )
                ) {
                    Column(
                        modifier = Modifier.padding(16.dp),
                        verticalArrangement = Arrangement.spacedBy(12.dp)
                    ) {
                        group.title?.let {
                            Text(
                                text = it,
                                style = MaterialTheme.typography.titleSmall
                            )
                        }
                        group.controls.forEach { control ->
                            AdjustmentPill(control, onAdjustmentSelected)
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun AdjustmentPill(
    adjustment: RemoteAdjustment,
    onAdjustmentSelected: (RemoteAdjustment) -> Unit,
    modifier: Modifier = Modifier
) {
    Surface(
        modifier = modifier.fillMaxWidth(),
        onClick = { if (adjustment.isEnabled) onAdjustmentSelected(adjustment) },
        tonalElevation = if (adjustment.isEnabled) 3.dp else 0.dp
    ) {
        Column(
            modifier = Modifier
                .padding(vertical = 12.dp, horizontal = 16.dp)
                .alpha(if (adjustment.isEnabled) 1f else 0.4f),
            verticalArrangement = Arrangement.spacedBy(8.dp)
        ) {
            Text(
                text = adjustment.label,
                style = MaterialTheme.typography.bodyLarge
            )
            val helper = when (adjustment) {
                is RemoteAdjustment.Slider -> adjustment.helperText ?: "Value: ${adjustment.value.toInt()}"
                is RemoteAdjustment.Counter -> adjustment.helperText ?: "Count: ${adjustment.value}"
                is RemoteAdjustment.Toggle -> adjustment.helperText ?: if (adjustment.isChecked) "Enabled" else "Disabled"
            }
            Text(
                text = helper,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant
            )
        }
    }
}

@Composable
private fun TelemetrySection(
    cards: List<RemoteTelemetryCard>,
    onTelemetryClick: (RemoteTelemetryCard) -> Unit,
    modifier: Modifier = Modifier
) {
    Column(modifier = modifier.fillMaxWidth()) {
        Text(
            text = "Telemetry",
            style = MaterialTheme.typography.titleMedium
        )
        Spacer(modifier = Modifier.height(12.dp))

        Row(
            modifier = Modifier
                .horizontalScroll(rememberScrollState()),
            horizontalArrangement = Arrangement.spacedBy(16.dp)
        ) {
            cards.forEach { card ->
                TelemetryCard(card, onTelemetryClick)
            }
        }
    }
}

@Composable
private fun TelemetryCard(
    card: RemoteTelemetryCard,
    onTelemetryClick: (RemoteTelemetryCard) -> Unit,
    modifier: Modifier = Modifier
) {
    Card(
        modifier = modifier.width(200.dp),
        onClick = { onTelemetryClick(card) },
        colors = CardDefaults.cardColors(
            containerColor = MaterialTheme.colorScheme.surfaceColorAtElevation(3.dp)
        )
    ) {
        Column(
            modifier = Modifier.padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp)
        ) {
            Text(
                text = card.title,
                style = MaterialTheme.typography.titleSmall
            )
            when (card) {
                is RemoteTelemetryCard.Counter -> {
                    Text(
                        text = card.value.toString(),
                        style = MaterialTheme.typography.displaySmall
                    )
                    card.unit?.let {
                        Text(
                            text = it,
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant
                        )
                    }
                }

                is RemoteTelemetryCard.Gauge -> GaugeStub(card)

                is RemoteTelemetryCard.Text -> Text(
                    text = card.body,
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = 3,
                    overflow = TextOverflow.Ellipsis
                )
            }
        }
    }
}

@Composable
private fun GaugeStub(card: RemoteTelemetryCard.Gauge, modifier: Modifier = Modifier) {
    Box(
        modifier = modifier
            .size(120.dp)
            .background(MaterialTheme.colorScheme.primary.copy(alpha = 0.1f), MaterialTheme.shapes.medium),
        contentAlignment = Alignment.Center
    ) {
        Column(horizontalAlignment = Alignment.CenterHorizontally) {
            Icon(
                imageVector = Icons.Default.Speed,
                contentDescription = null,
                tint = MaterialTheme.colorScheme.primary
            )
            Spacer(modifier = Modifier.height(8.dp))
            Text(
                text = "${card.value.toInt()}${card.unit.orEmpty()}",
                style = MaterialTheme.typography.titleLarge
            )
            Text(
                text = "max ${card.max.toInt()}",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant
            )
        }
    }
}

@Preview(showBackground = true)
@Composable
private fun RemotePanelScreenPreview() {
    HandControlTheme {
        Surface {
            RemotePanelScreen(
                uiModel = previewRemotePanelModel()
            )
        }
    }
}

private fun previewRemotePanelModel(): RemotePanelUiModel {
    val quickActions = listOf(
        RemoteQuickAction(
            id = "power_on",
            title = "Power On",
            subtitle = "Boot the system",
            iconKey = "bolt",
            isEnabled = true,
            isBusy = false,
            requiresConfirmation = true,
            showOutput = true
        ),
        RemoteQuickAction(
            id = "power_off",
            title = "Power Off",
            subtitle = "Immediate shutdown",
            iconKey = "power_settings",
            isEnabled = true,
            isBusy = false,
            requiresConfirmation = true,
            showOutput = true
        ),
        RemoteQuickAction(
            id = "restart",
            title = "Restart",
            subtitle = "Safe reboot",
            iconKey = "restart",
            isEnabled = false,
            isBusy = false,
            requiresConfirmation = true,
            showOutput = true
        )
    )

    val adjustments = listOf(
        RemoteAdjustmentGroup(
            id = "audio",
            title = "Audio",
            controls = listOf(
                RemoteAdjustment.Slider(
                    id = "volume",
                    label = "Volume",
                    isEnabled = true,
                    value = 45f,
                    range = 0f..100f,
                    step = 1f,
                    helperText = "Tap to fine tune"
                ),
                RemoteAdjustment.Toggle(
                    id = "mute",
                    label = "Mute",
                    isEnabled = true,
                    isChecked = false,
                    helperText = null
                )
            )
        ),
        RemoteAdjustmentGroup(
            id = "lights",
            title = "Lighting",
            controls = listOf(
                RemoteAdjustment.Slider(
                    id = "brightness",
                    label = "Brightness",
                    isEnabled = true,
                    value = 65f,
                    range = 0f..100f,
                    step = 5f,
                    helperText = null
                )
            )
        )
    )

    val telemetry = listOf(
        RemoteTelemetryCard.Gauge(
            id = "cpu_usage",
            title = "CPU Load",
            isLoading = false,
            lastUpdatedTimestamp = null,
            value = 72f,
            min = 0f,
            max = 100f,
            unit = "%",
            trend = RemoteTelemetryCard.Trend(
                direction = RemoteTelemetryCard.Trend.Direction.UP,
                magnitude = 4f
            )
        ),
        RemoteTelemetryCard.Counter(
            id = "uptime",
            title = "Uptime",
            isLoading = false,
            lastUpdatedTimestamp = null,
            value = 12345,
            unit = "s",
            delta = 45
        ),
        RemoteTelemetryCard.Text(
            id = "status",
            title = "Status",
            isLoading = false,
            lastUpdatedTimestamp = null,
            body = "All systems nominal. Last check 2 minutes ago."
        )
    )

    return RemotePanelUiModel(
        quickActions = quickActions,
        adjustments = adjustments,
        telemetry = telemetry
    )
}

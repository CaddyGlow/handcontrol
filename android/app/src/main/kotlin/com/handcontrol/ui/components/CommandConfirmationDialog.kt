package com.handcontrol.ui.components

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Warning
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.tooling.preview.Preview
import androidx.compose.ui.unit.dp
import com.handcontrol.data.commands.Command
import com.handcontrol.data.commands.ParameterType
import com.handcontrol.ui.theme.HandControlTheme

/**
 * Confirmation dialog shown before executing commands with requires_confirmation = true
 */
@Composable
fun CommandConfirmationDialog(
    command: Command,
    parameters: Map<String, String> = emptyMap(),
    onConfirm: () -> Unit,
    onDismiss: () -> Unit,
    modifier: Modifier = Modifier
) {
    AlertDialog(
        onDismissRequest = onDismiss,
        icon = {
            Icon(
                imageVector = Icons.Default.Warning,
                contentDescription = null,
                tint = MaterialTheme.colorScheme.error
            )
        },
        title = {
            Text(text = "Execute Command?")
        },
        text = {
            Column {
                Text(
                    text = command.name,
                    style = MaterialTheme.typography.titleMedium,
                    fontWeight = FontWeight.Bold
                )

                if (command.description.isNotBlank()) {
                    Spacer(modifier = Modifier.height(8.dp))
                    Text(
                        text = command.description,
                        style = MaterialTheme.typography.bodyMedium
                    )
                }

                if (parameters.isNotEmpty()) {
                    Spacer(modifier = Modifier.height(12.dp))
                    Text(
                        text = "Parameters:",
                        style = MaterialTheme.typography.labelMedium,
                        fontWeight = FontWeight.Bold
                    )
                    parameters.forEach { (key, value) ->
                        Text(
                            text = "$key: $value",
                            style = MaterialTheme.typography.bodySmall,
                            modifier = Modifier.padding(start = 8.dp, top = 2.dp)
                        )
                    }
                }
            }
        },
        confirmButton = {
            TextButton(
                onClick = {
                    onDismiss()
                    onConfirm()
                }
            ) {
                Text("Execute")
            }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) {
                Text("Cancel")
            }
        },
        modifier = modifier
    )
}

@Preview(showBackground = true)
@Composable
private fun CommandConfirmationDialogPreview() {
    HandControlTheme {
        CommandConfirmationDialog(
            command = Command(
                id = "shutdown",
                name = "Shutdown System",
                description = "This will shut down the computer immediately",
                icon = "power",
                tags = listOf("system", "power"),
                parameters = emptyList(),
                requiresConfirmation = true,
                showOutput = true
            ),
            onConfirm = {},
            onDismiss = {}
        )
    }
}

@Preview(showBackground = true)
@Composable
private fun CommandConfirmationDialogWithParametersPreview() {
    HandControlTheme {
        CommandConfirmationDialog(
            command = Command(
                id = "delete-logs",
                name = "Delete System Logs",
                description = "Permanently delete system logs",
                icon = "delete",
                tags = listOf("system"),
                parameters = emptyList(),
                requiresConfirmation = true,
                showOutput = true
            ),
            parameters = mapOf(
                "log_type" to "system",
                "confirm" to "DELETE"
            ),
            onConfirm = {},
            onDismiss = {}
        )
    }
}

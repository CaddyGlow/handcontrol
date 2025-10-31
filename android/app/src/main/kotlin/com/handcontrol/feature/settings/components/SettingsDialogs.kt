package com.handcontrol.feature.settings.components

import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp

@Composable
fun ClearDataConfirmationDialog(
    onConfirm: () -> Unit,
    onDismiss: () -> Unit
) {
    var confirmationText by remember { mutableStateOf("") }

    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Clear All Data") },
        text = {
            Column {
                Text("This will permanently delete:")
                Text("- All enrolled servers")
                Text("- Client certificates")
                Text("- App settings")
                Spacer(modifier = Modifier.height(16.dp))
                Text("Type DELETE ALL to confirm:")
                OutlinedTextField(
                    value = confirmationText,
                    onValueChange = { confirmationText = it },
                    singleLine = true,
                    modifier = Modifier.fillMaxWidth()
                )
            }
        },
        confirmButton = {
            Button(
                onClick = onConfirm,
                enabled = confirmationText == "DELETE ALL",
                colors = ButtonDefaults.buttonColors(
                    containerColor = MaterialTheme.colorScheme.error
                )
            ) {
                Text("Delete")
            }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) {
                Text("Cancel")
            }
        }
    )
}

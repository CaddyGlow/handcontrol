package com.handcontrol.ui.components

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.delay
import com.handcontrol.data.commands.Command
import com.handcontrol.data.commands.ParameterInputState
import com.handcontrol.data.commands.ValidationResult
import com.handcontrol.data.commands.defaultValueFor
import com.handcontrol.data.commands.toProtoParameter
import com.handcontrol.data.commands.validateParameterValue
import com.handcontrol.ui.components.parameters.ParameterInput

/**
 * Dialog for quickly executing a command with a single parameter.
 * Shows the parameter input directly in a dialog instead of navigating to a full screen.
 *
 * @param command The command to execute (must have exactly 1 parameter)
 * @param initialValue The initial value for the parameter
 * @param isLoadingDefault Whether a dynamic default is being loaded
 * @param onExecute Callback when user confirms execution with the parameter value
 * @param onRefreshDefault Callback to refresh dynamic default value
 * @param onDismiss Callback when dialog is dismissed
 */
@Composable
fun SingleParameterDialog(
    command: Command,
    initialValue: String,
    isLoadingDefault: Boolean = false,
    onExecute: (String) -> Unit,
    onRefreshDefault: () -> Unit = {},
    onDismiss: () -> Unit,
    modifier: Modifier = Modifier
) {
    require(command.parameters.size == 1) { "SingleParameterDialog requires exactly 1 parameter" }

    val parameter = command.parameters.first()
    val protoParam = parameter.toProtoParameter()

    var parameterState by remember(initialValue) {
        mutableStateOf(
            ParameterInputState(
                parameter = protoParam,
                currentValue = initialValue,
                validationResult = ValidationResult.Pending,
                isDirty = false
            )
        )
    }

    // Validate on value change
    LaunchedEffect(parameterState.currentValue) {
        val validationResult = validateParameterValue(protoParam, parameterState.currentValue)
        if (parameterState.validationResult != validationResult) {
            parameterState = parameterState.copy(validationResult = validationResult)
        }
    }

    val isValid = parameterState.validationResult is ValidationResult.Valid
    val isAutoExecuteType = parameter.type == com.handcontrol.data.commands.ParameterType.TOGGLE ||
                            parameter.type == com.handcontrol.data.commands.ParameterType.SLIDER

    // Auto-execute for TOGGLE and SLIDER types with debouncing for SLIDER
    LaunchedEffect(parameterState.currentValue, parameterState.isDirty) {
        if (parameterState.isDirty && isValid && !isLoadingDefault && isAutoExecuteType) {
            if (parameter.type == com.handcontrol.data.commands.ParameterType.SLIDER) {
                // Debounce slider changes to avoid excessive executions
                delay(300)
            }
            onExecute(parameterState.currentValue)
        }
    }

    AlertDialog(
        onDismissRequest = onDismiss,
        title = {
            Text(text = command.name)
        },
        text = {
            Column(
                modifier = Modifier.fillMaxWidth()
            ) {
                Text(
                    text = command.description,
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )

                Spacer(modifier = Modifier.height(16.dp))

                ParameterInput(
                    parameter = protoParam,
                    inputState = parameterState,
                    onValueChange = { newValue ->
                        parameterState = parameterState.copy(
                            currentValue = newValue,
                            isDirty = true
                        )
                    },
                    onRefreshDefault = if (parameter.defaultValueCommand != null) {
                        onRefreshDefault
                    } else {
                        null
                    },
                    isLoadingDefault = isLoadingDefault
                )
            }
        },
        confirmButton = {
            if (isAutoExecuteType) {
                // For TOGGLE/SLIDER: Show "Done" button to close dialog
                TextButton(onClick = onDismiss) {
                    Text("Done")
                }
            } else {
                // For TEXT/DROPDOWN: Show "Execute" button
                TextButton(
                    onClick = { onExecute(parameterState.currentValue) },
                    enabled = isValid && !isLoadingDefault
                ) {
                    Text("Execute")
                }
            }
        },
        dismissButton = {
            if (!isAutoExecuteType) {
                TextButton(onClick = onDismiss) {
                    Text("Cancel")
                }
            }
        },
        modifier = modifier
    )
}

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
        val validationResult = validateParameter(protoParam, parameterState.currentValue)
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

/**
 * Extension function to convert domain parameter to proto parameter for validation
 */
private fun com.handcontrol.data.commands.CommandParameter.toProtoParameter(): com.handcontrol.grpc.Parameter {
    return com.handcontrol.grpc.Parameter.newBuilder().apply {
        name = this@toProtoParameter.name
        type = when (this@toProtoParameter.type) {
            com.handcontrol.data.commands.ParameterType.SLIDER -> com.handcontrol.grpc.ParameterType.PARAMETER_TYPE_SLIDER
            com.handcontrol.data.commands.ParameterType.TEXT -> com.handcontrol.grpc.ParameterType.PARAMETER_TYPE_TEXT
            com.handcontrol.data.commands.ParameterType.TOGGLE -> com.handcontrol.grpc.ParameterType.PARAMETER_TYPE_TOGGLE
            com.handcontrol.data.commands.ParameterType.DROPDOWN -> com.handcontrol.grpc.ParameterType.PARAMETER_TYPE_DROPDOWN
            com.handcontrol.data.commands.ParameterType.UNSPECIFIED -> com.handcontrol.grpc.ParameterType.PARAMETER_TYPE_UNSPECIFIED
        }
        description = this@toProtoParameter.description
        this@toProtoParameter.min?.let { min = it }
        this@toProtoParameter.max?.let { max = it }
        this@toProtoParameter.defaultValue?.let { defaultValue = it }
        addAllOptions(this@toProtoParameter.options)
        this@toProtoParameter.validation?.let { validation = it }
        this@toProtoParameter.labelOn?.let { labelOn = it }
        this@toProtoParameter.labelOff?.let { labelOff = it }
    }.build()
}

/**
 * Validate parameter value
 */
private fun validateParameter(
    parameter: com.handcontrol.grpc.Parameter,
    value: String
): ValidationResult {
    return when (parameter.type) {
        com.handcontrol.grpc.ParameterType.PARAMETER_TYPE_SLIDER -> {
            val numValue = value.toIntOrNull()
            when {
                numValue == null -> ValidationResult.Invalid("Must be a number")
                parameter.hasMin() && numValue < parameter.min -> ValidationResult.Invalid("Minimum value is ${parameter.min}")
                parameter.hasMax() && numValue > parameter.max -> ValidationResult.Invalid("Maximum value is ${parameter.max}")
                else -> ValidationResult.Valid
            }
        }
        com.handcontrol.grpc.ParameterType.PARAMETER_TYPE_TEXT -> {
            if (parameter.hasValidation()) {
                try {
                    val regex = Regex(parameter.validation)
                    if (regex.matches(value)) {
                        ValidationResult.Valid
                    } else {
                        ValidationResult.Invalid("Invalid format")
                    }
                } catch (e: Exception) {
                    ValidationResult.Valid
                }
            } else {
                ValidationResult.Valid
            }
        }
        com.handcontrol.grpc.ParameterType.PARAMETER_TYPE_TOGGLE -> {
            if (value == "true" || value == "false") {
                ValidationResult.Valid
            } else {
                ValidationResult.Invalid("Must be true or false")
            }
        }
        com.handcontrol.grpc.ParameterType.PARAMETER_TYPE_DROPDOWN -> {
            if (parameter.optionsList.contains(value)) {
                ValidationResult.Valid
            } else {
                ValidationResult.Invalid("Invalid option")
            }
        }
        else -> ValidationResult.Valid
    }
}

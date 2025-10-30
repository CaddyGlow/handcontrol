package com.handcontrol.ui.components.parameters

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material.icons.filled.Warning
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import com.handcontrol.data.commands.ParameterInputState
import com.handcontrol.data.commands.ValidationResult
import com.handcontrol.grpc.Parameter
import com.handcontrol.grpc.ParameterType

/**
 * Renders the appropriate parameter input based on parameter type
 */
@Composable
fun ParameterInput(
    parameter: Parameter,
    inputState: ParameterInputState,
    onValueChange: (String) -> Unit,
    modifier: Modifier = Modifier,
    onRefreshDefault: (() -> Unit)? = null,
    isLoadingDefault: Boolean = false
) {
    Column(
        modifier = modifier
            .fillMaxWidth()
            .padding(vertical = 8.dp)
    ) {
        // Parameter label with refresh button
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.SpaceBetween,
            verticalAlignment = Alignment.CenterVertically
        ) {
            Column(modifier = Modifier.weight(1f)) {
                ParameterLabel(
                    name = parameter.name,
                    description = parameter.description
                )
            }

            // Show refresh button if dynamic default is available
            if (onRefreshDefault != null) {
                IconButton(
                    onClick = onRefreshDefault,
                    enabled = !isLoadingDefault
                ) {
                    if (isLoadingDefault) {
                        CircularProgressIndicator(
                            modifier = Modifier.size(20.dp),
                            strokeWidth = 2.dp
                        )
                    } else {
                        Icon(
                            imageVector = Icons.Default.Refresh,
                            contentDescription = "Refresh default value",
                            tint = MaterialTheme.colorScheme.primary
                        )
                    }
                }
            }
        }

        Spacer(modifier = Modifier.height(8.dp))

        // Type-specific input control with loading overlay
        Box {
            when (parameter.type) {
                ParameterType.PARAMETER_TYPE_SLIDER -> {
                    SliderParameterInput(
                        parameter = parameter,
                        currentValue = inputState.currentValue,
                        onValueChange = onValueChange,
                        isError = inputState.validationResult is ValidationResult.Invalid
                    )
                }
                ParameterType.PARAMETER_TYPE_TEXT -> {
                    TextParameterInput(
                        parameter = parameter,
                        currentValue = inputState.currentValue,
                        onValueChange = onValueChange,
                        isError = inputState.validationResult is ValidationResult.Invalid
                    )
                }
                ParameterType.PARAMETER_TYPE_TOGGLE -> {
                    ToggleParameterInput(
                        parameter = parameter,
                        currentValue = inputState.currentValue,
                        onValueChange = onValueChange
                    )
                }
                ParameterType.PARAMETER_TYPE_DROPDOWN -> {
                    DropdownParameterInput(
                        parameter = parameter,
                        currentValue = inputState.currentValue,
                        onValueChange = onValueChange,
                        isError = inputState.validationResult is ValidationResult.Invalid
                    )
                }
                else -> {
                    Text(
                        text = "Unsupported parameter type: ${parameter.type}",
                        color = MaterialTheme.colorScheme.error,
                        style = MaterialTheme.typography.bodyMedium
                    )
                }
            }

            // Loading overlay
            if (isLoadingDefault) {
                Box(
                    modifier = Modifier
                        .matchParentSize()
                        .background(MaterialTheme.colorScheme.surface.copy(alpha = 0.7f)),
                    contentAlignment = Alignment.Center
                ) {
                    CircularProgressIndicator(modifier = Modifier.size(24.dp))
                }
            }
        }

        // Validation message
        if (inputState.validationResult is ValidationResult.Invalid && inputState.isDirty) {
            ValidationMessage(inputState.validationResult.message)
        }
    }
}

@Composable
private fun ParameterLabel(name: String, description: String?) {
    Column {
        Text(
            text = name.replaceFirstChar { it.titlecase() },
            style = MaterialTheme.typography.labelLarge,
            fontWeight = FontWeight.Medium
        )
        if (!description.isNullOrBlank()) {
            Spacer(modifier = Modifier.height(4.dp))
            Text(
                text = description,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant
            )
        }
    }
}

@Composable
private fun ValidationMessage(message: String) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(top = 4.dp),
        verticalAlignment = Alignment.CenterVertically
    ) {
        Icon(
            imageVector = Icons.Default.Warning,
            contentDescription = "Error",
            tint = MaterialTheme.colorScheme.error,
            modifier = Modifier.size(16.dp)
        )
        Spacer(modifier = Modifier.width(4.dp))
        Text(
            text = message,
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.error
        )
    }
}

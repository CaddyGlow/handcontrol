package com.handcontrol.ui.components.parameters

import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import com.handcontrol.grpc.CapabilityParameter

@Composable
fun TextParameterInput(
    parameter: CapabilityParameter,
    currentValue: String,
    onValueChange: (String) -> Unit,
    isError: Boolean,
    modifier: Modifier = Modifier
) {
    OutlinedTextField(
        value = currentValue,
        onValueChange = onValueChange,
        modifier = modifier.fillMaxWidth(),
        label = { Text(parameter.name) },
        placeholder = {
            Text(
                if (parameter.hasDefaultValue() && parameter.defaultValue.isNotBlank()) {
                    parameter.defaultValue
                } else {
                    "Enter ${parameter.name}..."
                }
            )
        },
        supportingText = if (parameter.hasValidation() && parameter.validation.isNotBlank()) {
            { Text("Format: ${parameter.validation}") }
        } else null,
        isError = isError,
        singleLine = true,
        keyboardOptions = KeyboardOptions(
            keyboardType = detectKeyboardType(parameter.validation),
            imeAction = ImeAction.Done
        ),
        keyboardActions = KeyboardActions(
            onDone = { /* Focus next field or execute */ }
        )
    )
}

private fun detectKeyboardType(validation: String?): KeyboardType {
    if (validation.isNullOrBlank()) return KeyboardType.Text

    return when {
        validation.contains("\\d") || validation.contains("[0-9]") -> KeyboardType.Number
        validation.contains("@") -> KeyboardType.Email
        validation.contains("://") -> KeyboardType.Uri
        else -> KeyboardType.Text
    }
}

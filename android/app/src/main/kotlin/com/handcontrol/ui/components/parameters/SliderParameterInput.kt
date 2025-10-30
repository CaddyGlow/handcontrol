package com.handcontrol.ui.components.parameters

import androidx.compose.foundation.layout.*
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.Remove
import androidx.compose.material3.*
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import com.handcontrol.grpc.Parameter

@Composable
fun SliderParameterInput(
    parameter: Parameter,
    currentValue: String,
    onValueChange: (String) -> Unit,
    isError: Boolean,
    modifier: Modifier = Modifier
) {
    val min = if (parameter.hasMin()) parameter.min else 0
    val max = if (parameter.hasMax()) parameter.max else 100
    val value = currentValue.toIntOrNull() ?: min

    Column(modifier = modifier.fillMaxWidth()) {
        // Current value display
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.SpaceBetween,
            verticalAlignment = Alignment.CenterVertically
        ) {
            Text(
                text = "Current: $value",
                style = MaterialTheme.typography.bodyMedium,
                fontWeight = FontWeight.Medium
            )

            // Step buttons for fine control
            Row {
                IconButton(
                    onClick = {
                        if (value > min) onValueChange((value - 1).toString())
                    },
                    enabled = value > min
                ) {
                    Icon(Icons.Default.Remove, "Decrease")
                }
                IconButton(
                    onClick = {
                        if (value < max) onValueChange((value + 1).toString())
                    },
                    enabled = value < max
                ) {
                    Icon(Icons.Default.Add, "Increase")
                }
            }
        }

        Spacer(modifier = Modifier.height(8.dp))

        // Slider
        Slider(
            value = value.toFloat(),
            onValueChange = { onValueChange(it.toInt().toString()) },
            valueRange = min.toFloat()..max.toFloat(),
            steps = if (max - min < 100) (max - min - 1).coerceAtLeast(0) else 0,
            colors = SliderDefaults.colors(
                thumbColor = if (isError)
                    MaterialTheme.colorScheme.error
                else
                    MaterialTheme.colorScheme.primary
            ),
            modifier = Modifier.fillMaxWidth()
        )

        // Min/Max labels
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.SpaceBetween
        ) {
            Text(
                text = min.toString(),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant
            )
            Text(
                text = max.toString(),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant
            )
        }
    }
}

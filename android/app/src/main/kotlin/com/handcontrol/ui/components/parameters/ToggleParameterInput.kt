package com.handcontrol.ui.components.parameters

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import com.handcontrol.grpc.Parameter

@Composable
fun ToggleParameterInput(
    parameter: Parameter,
    currentValue: String,
    onValueChange: (String) -> Unit,
    modifier: Modifier = Modifier
) {
    val isChecked = currentValue.toBoolean()

    val labelOn = if (parameter.hasLabelOn() && parameter.labelOn.isNotBlank()) {
        parameter.labelOn
    } else {
        "On"
    }

    val labelOff = if (parameter.hasLabelOff() && parameter.labelOff.isNotBlank()) {
        parameter.labelOff
    } else {
        "Off"
    }

    Row(
        modifier = modifier
            .fillMaxWidth()
            .clip(MaterialTheme.shapes.medium)
            .clickable { onValueChange((!isChecked).toString()) }
            .padding(vertical = 12.dp, horizontal = 16.dp),
        horizontalArrangement = Arrangement.SpaceBetween,
        verticalAlignment = Alignment.CenterVertically
    ) {
        Column(modifier = Modifier.weight(1f)) {
            Text(
                text = if (isChecked) labelOn else labelOff,
                style = MaterialTheme.typography.bodyLarge,
                fontWeight = FontWeight.Medium
            )
            if (parameter.description.isNotBlank()) {
                Spacer(modifier = Modifier.height(4.dp))
                Text(
                    text = parameter.description,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )
            }
        }

        Switch(
            checked = isChecked,
            onCheckedChange = { onValueChange(it.toString()) }
        )
    }
}

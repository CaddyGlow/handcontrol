package com.handcontrol.data.commands

import com.handcontrol.grpc.CapabilityParameter
import com.handcontrol.grpc.CapabilityParameterType

/**
 * Convert a [CommandParameter] into its protobuf representation for validation/UI helpers.
 */
fun CommandParameter.toProtoParameter(): CapabilityParameter {
    val protoType = when (type) {
        ParameterType.SLIDER -> CapabilityParameterType.CAPABILITY_PARAMETER_TYPE_SLIDER
        ParameterType.TEXT -> CapabilityParameterType.CAPABILITY_PARAMETER_TYPE_TEXT
        ParameterType.TOGGLE -> CapabilityParameterType.CAPABILITY_PARAMETER_TYPE_TOGGLE
        ParameterType.DROPDOWN -> CapabilityParameterType.CAPABILITY_PARAMETER_TYPE_DROPDOWN
        ParameterType.UNSPECIFIED -> CapabilityParameterType.CAPABILITY_PARAMETER_TYPE_UNSPECIFIED
    }

    return CapabilityParameter.newBuilder()
        .setName(name)
        .setType(protoType)
        .setDescription(description)
        .apply {
            min?.let { setMin(it) }
            max?.let { setMax(it) }
            defaultValue?.let { setDefaultValue(it) }
            validation?.let { setValidation(it) }
            labelOn?.let { setLabelOn(it) }
            labelOff?.let { setLabelOff(it) }
            defaultValueCommand?.let { setDefaultValueCommand(it) }
            defaultValuePattern?.let { setDefaultValuePattern(it) }
            addAllOptions(options)
        }
        .build()
}

/**
 * Provides a reasonable default value for the given parameter type when no default is supplied.
 */
fun defaultValueFor(type: ParameterType): String {
    return when (type) {
        ParameterType.SLIDER -> "0"
        ParameterType.TEXT -> ""
        ParameterType.TOGGLE -> "false"
        ParameterType.DROPDOWN -> ""
        ParameterType.UNSPECIFIED -> ""
    }
}

/**
 * Validate a value for the provided capability parameter.
 */
fun validateParameterValue(
    parameter: CapabilityParameter,
    value: String
): ValidationResult {
    return when (parameter.type) {
        CapabilityParameterType.CAPABILITY_PARAMETER_TYPE_SLIDER -> {
            val numValue = value.toIntOrNull()
                ?: return ValidationResult.Invalid("Must be a number")

            if (parameter.hasMin() && numValue < parameter.min) {
                return ValidationResult.Invalid("Minimum value is ${parameter.min}")
            }
            if (parameter.hasMax() && numValue > parameter.max) {
                return ValidationResult.Invalid("Maximum value is ${parameter.max}")
            }
            ValidationResult.Valid
        }
        CapabilityParameterType.CAPABILITY_PARAMETER_TYPE_TEXT -> {
            if (parameter.hasValidation() && parameter.validation.isNotBlank()) {
                try {
                    val regex = Regex(parameter.validation)
                    if (!regex.matches(value)) {
                        return ValidationResult.Invalid("Invalid format")
                    }
                } catch (_: Exception) {
                    return ValidationResult.Invalid("Invalid validation pattern")
                }
            }
            ValidationResult.Valid
        }
        CapabilityParameterType.CAPABILITY_PARAMETER_TYPE_TOGGLE -> {
            if (value == "true" || value == "false") {
                ValidationResult.Valid
            } else {
                ValidationResult.Invalid("Must be true or false")
            }
        }
        CapabilityParameterType.CAPABILITY_PARAMETER_TYPE_DROPDOWN -> {
            if (parameter.optionsList.contains(value)) {
                ValidationResult.Valid
            } else {
                ValidationResult.Invalid("Invalid selection")
            }
        }
        CapabilityParameterType.CAPABILITY_PARAMETER_TYPE_UNSPECIFIED,
        CapabilityParameterType.UNRECOGNIZED -> {
            ValidationResult.Invalid("Unknown parameter type")
        }
    }
}

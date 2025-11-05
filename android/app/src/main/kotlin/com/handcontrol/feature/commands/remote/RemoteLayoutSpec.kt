package com.handcontrol.feature.commands.remote

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable

/**
 * Persisted definition describing how the remote control panel should be laid out.
 * It references existing commands (and their parameters) while allowing the user
 * to organise controls into sections and tweak basic presentation options.
 */
@Serializable
data class RemoteLayoutSpec(
    val sections: List<SectionSpec> = emptyList()
) {
    @Serializable
    data class SectionSpec(
        val id: String,
        val type: SectionType,
        val title: String? = null,
        val entries: List<RemoteLayoutEntry> = emptyList()
    )

    @Serializable
    enum class SectionType {
        @SerialName("quick_actions")
        QUICK_ACTIONS,

        @SerialName("adjustments")
        ADJUSTMENTS,

        @SerialName("telemetry")
        TELEMETRY
    }
}

@Serializable
sealed class RemoteLayoutEntry {
    abstract val id: String

    @Serializable
    @SerialName("quick_action")
    data class QuickActionEntry(
        override val id: String,
        val commandId: String,
        val labelOverride: String? = null,
        val iconOverride: String? = null,
        val requiresConfirmationOverride: Boolean? = null,
        val showOutputOverride: Boolean? = null
    ) : RemoteLayoutEntry()

    @Serializable
    @SerialName("adjustment")
    data class AdjustmentEntry(
        override val id: String,
        val commandId: String,
        val parameterName: String,
        val labelOverride: String? = null,
        val control: AdjustmentControl = AdjustmentControl.AUTO,
        val slider: SliderOverrides? = null,
        val toggle: ToggleOverrides? = null,
        val counter: CounterOverrides? = null,
        val helperText: String? = null
    ) : RemoteLayoutEntry()

    @Serializable
    enum class AdjustmentControl {
        @SerialName("auto")
        AUTO,

        @SerialName("slider")
        SLIDER,

        @SerialName("toggle")
        TOGGLE,

        @SerialName("counter")
        COUNTER
    }

    @Serializable
    data class SliderOverrides(
        val min: Float? = null,
        val max: Float? = null,
        val step: Float? = null
    )

    @Serializable
    data class ToggleOverrides(
        val checkedLabel: String? = null,
        val uncheckedLabel: String? = null
    )

    @Serializable
    data class CounterOverrides(
        val min: Int? = null,
        val max: Int? = null
    )

    @Serializable
    @SerialName("telemetry")
    data class TelemetryEntry(
        override val id: String,
        val sourceCommandId: String,
        val titleOverride: String? = null,
        val display: TelemetryDisplay = TelemetryDisplay.COUNTER,
        val unit: String? = null,
        val min: Float? = null,
        val max: Float? = null,
        val textTemplate: String? = null
    ) : RemoteLayoutEntry()

    @Serializable
    enum class TelemetryDisplay {
        @SerialName("counter")
        COUNTER,

        @SerialName("gauge")
        GAUGE,

        @SerialName("text")
        TEXT
    }
}

package com.handcontrol.feature.commands.remote

import androidx.compose.runtime.Immutable

/**
 * Represents the data the remote-style control panel needs to render.
 * This is intentionally interaction-agnostic so the view model can
 * populate it while the UI layer decides how to surface callbacks.
 */
@Immutable
data class RemotePanelUiModel(
    val quickActions: List<RemoteQuickAction>,
    val adjustments: List<RemoteAdjustmentGroup>,
    val telemetry: List<RemoteTelemetryCard>
) {
    val hasContent: Boolean
        get() = quickActions.isNotEmpty() || adjustments.isNotEmpty() || telemetry.isNotEmpty()

    companion object {
        fun empty() = RemotePanelUiModel(
            quickActions = emptyList(),
            adjustments = emptyList(),
            telemetry = emptyList()
        )
    }
}

@Immutable
data class RemoteQuickAction(
    val id: String,
    val title: String,
    val subtitle: String?,
    val iconKey: String?,
    val isEnabled: Boolean,
    val isBusy: Boolean,
    val requiresConfirmation: Boolean,
    val showOutput: Boolean
)

@Immutable
data class RemoteAdjustmentGroup(
    val id: String,
    val title: String?,
    val controls: List<RemoteAdjustment>
)

@Immutable
sealed interface RemoteAdjustment {
    val id: String
    val label: String
    val isEnabled: Boolean

    @Immutable
    data class Slider(
        override val id: String,
        override val label: String,
        override val isEnabled: Boolean,
        val value: Float,
        val range: ClosedFloatingPointRange<Float>,
        val step: Float?,
        val helperText: String? = null
    ) : RemoteAdjustment

    @Immutable
    data class Counter(
        override val id: String,
        override val label: String,
        override val isEnabled: Boolean,
        val value: Int,
        val min: Int?,
        val max: Int?,
        val helperText: String? = null
    ) : RemoteAdjustment

    @Immutable
    data class Toggle(
        override val id: String,
        override val label: String,
        override val isEnabled: Boolean,
        val isChecked: Boolean,
        val helperText: String? = null
    ) : RemoteAdjustment
}

@Immutable
sealed interface RemoteTelemetryCard {
    val id: String
    val title: String
    val isLoading: Boolean
    val lastUpdatedTimestamp: Long?

    @Immutable
    data class Gauge(
        override val id: String,
        override val title: String,
        override val isLoading: Boolean,
        override val lastUpdatedTimestamp: Long?,
        val value: Float,
        val min: Float,
        val max: Float,
        val unit: String?,
        val trend: Trend?
    ) : RemoteTelemetryCard

    @Immutable
    data class Counter(
        override val id: String,
        override val title: String,
        override val isLoading: Boolean,
        override val lastUpdatedTimestamp: Long?,
        val value: Long,
        val unit: String?,
        val delta: Long?
    ) : RemoteTelemetryCard

    @Immutable
    data class Text(
        override val id: String,
        override val title: String,
        override val isLoading: Boolean,
        override val lastUpdatedTimestamp: Long?,
        val body: String
    ) : RemoteTelemetryCard

    @Immutable
    data class Trend(
        val direction: Direction,
        val magnitude: Float
    ) {
        enum class Direction { UP, DOWN, STEADY }
    }
}

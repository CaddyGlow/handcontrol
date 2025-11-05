package com.handcontrol.feature.commands.remote

import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.getValue
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.composed
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.input.pointer.consumePositionChange
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.LayoutCoordinates
import androidx.compose.ui.layout.boundsInRoot
import androidx.compose.ui.layout.onGloballyPositioned
import kotlinx.coroutines.channels.BufferOverflow
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.SharedFlow
import androidx.compose.foundation.gestures.detectDragGesturesAfterLongPress

/**
 * Simple drag and drop controller tailored for the remote layout builder.
 */
class DragDropState<T> internal constructor() {
    var draggingItem by mutableStateOf<T?>(null)
        private set
    var dragPosition by mutableStateOf(Offset.Zero)
        private set
    var isDragging by mutableStateOf(false)
        private set
    var currentTargetId by mutableStateOf<String?>(null)
        private set

    private val targets = mutableStateMapOf<String, Rect>()
    private val _results = MutableSharedFlow<DragResult<T>>(
        replay = 0,
        extraBufferCapacity = 1,
        onBufferOverflow = BufferOverflow.DROP_OLDEST
    )
    val results: SharedFlow<DragResult<T>> = _results

    fun startDrag(item: T, startPosition: Offset) {
        draggingItem = item
        dragPosition = startPosition
        isDragging = true
        updateCurrentTarget()
    }

    fun updateDrag(delta: Offset) {
        if (!isDragging) return
        dragPosition += delta
        updateCurrentTarget()
    }

    fun endDrag() {
        val item = draggingItem ?: kotlin.run {
            reset()
            return
        }
        val target = currentTargetId
        reset()
        _results.tryEmit(DragResult(item, target))
    }

    fun cancelDrag() {
        reset()
    }

    private fun reset() {
        draggingItem = null
        isDragging = false
        currentTargetId = null
    }

    fun registerTarget(id: String, rect: Rect) {
        targets[id] = rect
        updateCurrentTarget()
    }

    fun unregisterTarget(id: String) {
        targets.remove(id)
        if (currentTargetId == id) {
            currentTargetId = null
        }
    }

    private fun updateCurrentTarget() {
        if (!isDragging) {
            currentTargetId = null
            return
        }
        val containing = targets.entries.firstOrNull { it.value.contains(dragPosition) }
        currentTargetId = containing?.key
    }
}

data class DragResult<T>(val item: T, val targetId: String?)

@Composable
fun <T> rememberDragDropState(): DragDropState<T> {
    return remember { DragDropState() }
}

fun <T> Modifier.dragSource(
    state: DragDropState<T>,
    dataProvider: () -> T
): Modifier = composed {
    var coordinates by remember { mutableStateOf<LayoutCoordinates?>(null) }

    onGloballyPositioned { coordinates = it }
        .pointerInput(state) {
            detectDragGesturesAfterLongPress(
                onDragStart = { offset ->
                    val rootOffset = coordinates?.localToRoot(offset) ?: offset
                    state.startDrag(dataProvider(), rootOffset)
                },
                onDrag = { change, dragAmount ->
                    change.consumePositionChange()
                    state.updateDrag(dragAmount)
                },
                onDragEnd = {
                    state.endDrag()
                },
                onDragCancel = {
                    state.cancelDrag()
                }
            )
        }
}

fun <T> Modifier.dropTarget(
    state: DragDropState<T>,
    targetId: String
): Modifier = composed {
    onGloballyPositioned { coords ->
        state.registerTarget(targetId, coords.boundsInRoot())
    }.also { modifier ->
        DisposableEffect(state, targetId) {
            onDispose {
                state.unregisterTarget(targetId)
            }
        }
        modifier
    }
}

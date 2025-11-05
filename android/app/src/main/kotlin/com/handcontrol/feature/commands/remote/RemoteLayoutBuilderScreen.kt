package com.handcontrol.feature.commands.remote

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.ArrowDownward
import androidx.compose.material.icons.filled.ArrowUpward
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.handcontrol.data.commands.Command
import com.handcontrol.data.commands.CommandParameter
import com.handcontrol.data.commands.ParameterType
import java.util.UUID

private val QuickActionsSectionId = RemoteLayoutSpec.QUICK_ACTIONS_SECTION_ID
private val AdjustmentsSectionId = RemoteLayoutSpec.ADJUSTMENTS_SECTION_ID
private val TelemetrySectionId = RemoteLayoutSpec.TELEMETRY_SECTION_ID

@Composable
fun RemoteLayoutBuilderScreen(
    commands: List<Command>,
    initialSpec: RemoteLayoutSpec,
    onSpecChanged: (RemoteLayoutSpec) -> Unit,
    modifier: Modifier = Modifier
) {
    var workingSpec by remember(initialSpec) {
        mutableStateOf(initialSpec.ensureDefaultSections())
    }

    LaunchedEffect(initialSpec) {
        workingSpec = initialSpec.ensureDefaultSections()
    }

    fun updateSpec(transform: (RemoteLayoutSpec) -> RemoteLayoutSpec) {
        workingSpec = transform(workingSpec).ensureDefaultSections()
        onSpecChanged(workingSpec)
    }

    val dragState = rememberDragDropState<BuilderDragData>()
    val library = remember(commands) { buildLibrary(commands) }

    LaunchedEffect(dragState) {
        dragState.results.collect { result ->
            val data = result.item
            val targetId = result.targetId ?: return@collect

            when (data) {
                is BuilderDragData.LibraryItem -> {
                    updateSpec { spec ->
                        handleLibraryDrop(targetId, data.item, spec)
                    }
                }

                is BuilderDragData.CanvasItem -> {
                    updateSpec { spec ->
                        handleCanvasDrop(targetId, data, spec)
                    }
                }
            }
        }
    }

    Box(modifier = modifier.fillMaxSize()) {
        Row(
            modifier = Modifier
                .fillMaxSize(),
            horizontalArrangement = Arrangement.spacedBy(16.dp)
        ) {
            LibraryColumn(
                library = library,
                dragState = dragState,
                onQuickAdd = { item ->
                    updateSpec { spec ->
                        handleLibraryDrop(QuickActionsSectionId, item, spec)
                    }
                }
            )
            CanvasColumn(
                spec = workingSpec,
                commands = commands,
                dragState = dragState,
                onRemoveEntry = { sectionId, index ->
                    updateSpec {
                        it.updateSectionEntries(sectionId) { entries ->
                            entries.filterIndexed { idx, _ -> idx != index }
                        }
                    }
                },
                onMoveEntry = { sectionId, index, direction ->
                    updateSpec {
                        it.updateSectionEntries(sectionId) { entries ->
                            entries.move(index, direction)
                        }
                    }
                }
            )
        }

        if (dragState.isDragging && dragState.draggingItem != null) {
            val floatingLabel = when (val data = dragState.draggingItem) {
                is BuilderDragData.LibraryItem -> data.item.displayName
                is BuilderDragData.CanvasItem -> data.displayName
                else -> ""
            }

            Text(
                text = floatingLabel,
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurface,
                modifier = Modifier
                    .padding(8.dp)
                    .align(Alignment.TopStart)
                    .background(
                        color = MaterialTheme.colorScheme.surface,
                        shape = MaterialTheme.shapes.small
                    )
                    .padding(horizontal = 12.dp, vertical = 6.dp)
            )
        }
    }
}

@Composable
private fun LibraryColumn(
    library: BuilderLibrary,
    dragState: DragDropState<BuilderDragData>,
    onQuickAdd: (BuilderLibraryItem.QuickAction) -> Unit,
    modifier: Modifier = Modifier
) {
    Column(
        modifier = modifier
            .fillMaxHeight()
            .width(220.dp)
            .background(MaterialTheme.colorScheme.surfaceVariant, MaterialTheme.shapes.medium)
            .padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp)
    ) {
        Text(
            text = "Library",
            style = MaterialTheme.typography.titleMedium
        )

        LibrarySection(
            title = "Quick Actions",
            emptyMessage = "No stateless commands yet",
            items = library.quickActions,
            dragState = dragState,
            onQuickAdd = onQuickAdd
        ) { item ->
            item.command.name
        }

        LibrarySection(
            title = "Adjustments",
            emptyMessage = "No adjustable parameters",
            items = library.adjustments,
            dragState = dragState,
            onQuickAdd = { /* not used for adjustments */ }
        ) { item ->
            "${item.command.name} • ${item.parameter.name}"
        }

        LibrarySection(
            title = "Telemetry",
            emptyMessage = "Add commands to stream data",
            items = library.telemetry,
            dragState = dragState,
            onQuickAdd = { /* no quick-add */ }
        ) { item ->
            item.command.name
        }
    }
}

@Composable
private fun <T : BuilderLibraryItem> LibrarySection(
    title: String,
    emptyMessage: String,
    items: List<T>,
    dragState: DragDropState<BuilderDragData>,
    onQuickAdd: (BuilderLibraryItem.QuickAction) -> Unit,
    labelProvider: (T) -> String
) {
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Text(
            text = title,
            style = MaterialTheme.typography.titleSmall
        )
        if (items.isEmpty()) {
            Text(
                text = emptyMessage,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant
            )
        } else {
            items.forEach { item ->
                val display = labelProvider(item)
                Card(
                    colors = CardDefaults.cardColors(
                        containerColor = MaterialTheme.colorScheme.surface
                    ),
                    modifier = Modifier
                        .fillMaxWidth()
                        .dragSource(dragState) {
                            BuilderDragData.LibraryItem(item)
                        }
                ) {
                    Row(
                        modifier = Modifier
                            .fillMaxWidth()
                            .padding(12.dp),
                        horizontalArrangement = Arrangement.SpaceBetween,
                        verticalAlignment = Alignment.CenterVertically
                    ) {
                        Text(
                            text = display,
                            style = MaterialTheme.typography.bodyMedium,
                            maxLines = 2,
                            overflow = TextOverflow.Ellipsis
                        )
                        if (item is BuilderLibraryItem.QuickAction) {
                            IconButton(onClick = { onQuickAdd(item) }) {
                                Icon(
                                    imageVector = Icons.Default.Add,
                                    contentDescription = "Add"
                                )
                            }
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun CanvasColumn(
    spec: RemoteLayoutSpec,
    commands: List<Command>,
    dragState: DragDropState<BuilderDragData>,
    onRemoveEntry: (sectionId: String, index: Int) -> Unit,
    onMoveEntry: (sectionId: String, index: Int, direction: Int) -> Unit,
    modifier: Modifier = Modifier
) {
    Column(
        modifier = modifier
            .fillMaxSize()
            .padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp)
    ) {
        spec.sections.forEach { section ->
            SectionCanvas(
                section = section,
                commands = commands,
                dragState = dragState,
                onRemoveEntry = onRemoveEntry,
                onMoveEntry = onMoveEntry
            )
        }
    }
}

@Composable
private fun SectionCanvas(
    section: RemoteLayoutSpec.SectionSpec,
    commands: List<Command>,
    dragState: DragDropState<BuilderDragData>,
    onRemoveEntry: (String, Int) -> Unit,
    onMoveEntry: (String, Int, Int) -> Unit
) {
    val highlight = dragState.currentTargetId == section.id

    Card(
        colors = CardDefaults.cardColors(
            containerColor = MaterialTheme.colorScheme.surface
        ),
        modifier = Modifier
            .fillMaxWidth()
            .dropTarget(dragState, section.id)
            .border(
                width = if (highlight) 2.dp else 1.dp,
                color = if (highlight) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.outline,
                shape = MaterialTheme.shapes.large
            )
    ) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp)
        ) {
            Text(
                text = section.title ?: section.id.replaceFirstChar { it.uppercase() },
                style = MaterialTheme.typography.titleMedium
            )

            if (section.entries.isEmpty()) {
                Text(
                    text = "Drag items from the library",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.alpha(0.7f)
                )
            } else {
                val lastIndex = section.entries.lastIndex
                section.entries.forEachIndexed { index, entry ->
                CanvasEntryCard(
                    sectionId = section.id,
                    index = index,
                    entry = entry,
                    commands = commands,
                    dragState = dragState,
                    onRemoveEntry = onRemoveEntry,
                    onMoveEntry = onMoveEntry,
                    canMoveDown = index < lastIndex
                )
            }
            }
        }
    }
}

@Composable
private fun CanvasEntryCard(
    sectionId: String,
    index: Int,
    entry: RemoteLayoutEntry,
    commands: List<Command>,
    dragState: DragDropState<BuilderDragData>,
    onRemoveEntry: (String, Int) -> Unit,
    onMoveEntry: (String, Int, Int) -> Unit,
    canMoveDown: Boolean
) {
    val commandName = when (entry) {
        is RemoteLayoutEntry.QuickActionEntry -> commands.find { it.id == entry.commandId }?.name ?: entry.commandId
        is RemoteLayoutEntry.AdjustmentEntry -> commands.find { it.id == entry.commandId }?.name ?: entry.commandId
        is RemoteLayoutEntry.TelemetryEntry -> commands.find { it.id == entry.sourceCommandId }?.name ?: entry.sourceCommandId
    }

    Card(
        colors = CardDefaults.cardColors(
            containerColor = MaterialTheme.colorScheme.surfaceVariant
        ),
        modifier = Modifier
            .fillMaxWidth()
            .dragSource(dragState) {
                BuilderDragData.CanvasItem(
                    sectionId = sectionId,
                    index = index,
                    entry = entry
                )
            }
    ) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .padding(12.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp)
        ) {
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.SpaceBetween,
                modifier = Modifier.fillMaxWidth()
            ) {
                Column {
                    Text(
                        text = entry.displayTitle(commandName),
                        style = MaterialTheme.typography.titleSmall
                    )
                    Text(
                        text = entry.subtitle(commandName),
                        style = MaterialTheme.typography.bodySmall,
                        maxLines = 2,
                        overflow = TextOverflow.Ellipsis,
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                }
                Row(horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                    IconButton(onClick = { onMoveEntry(sectionId, index, -1) }, enabled = index > 0) {
                        Icon(
                            imageVector = Icons.Default.ArrowUpward,
                            contentDescription = "Move up"
                        )
                    }
                    IconButton(
                        onClick = { onMoveEntry(sectionId, index, +1) },
                        enabled = canMoveDown
                    ) {
                        Icon(
                            imageVector = Icons.Default.ArrowDownward,
                            contentDescription = "Move down"
                        )
                    }
                    IconButton(onClick = { onRemoveEntry(sectionId, index) }) {
                        Icon(
                            imageVector = Icons.Default.Delete,
                            contentDescription = "Remove"
                        )
                    }
                }
            }
        }
    }
}

private fun handleLibraryDrop(
    targetId: String,
    item: BuilderLibraryItem,
    spec: RemoteLayoutSpec
): RemoteLayoutSpec {
    return when (targetId) {
        QuickActionsSectionId -> {
            if (item is BuilderLibraryItem.QuickAction) {
                val entry = RemoteLayoutEntry.QuickActionEntry(
                    id = UUID.randomUUID().toString(),
                    commandId = item.command.id,
                    labelOverride = null,
                    iconOverride = item.command.icon.takeIf { it.isNotBlank() },
                    requiresConfirmationOverride = null,
                    showOutputOverride = null
                )
                spec.ensureSectionById(QuickActionsSectionId).appendEntry(QuickActionsSectionId, entry)
            } else {
                spec
            }
        }

        AdjustmentsSectionId -> {
            if (item is BuilderLibraryItem.Adjustment) {
                val entry = RemoteLayoutEntry.AdjustmentEntry(
                    id = UUID.randomUUID().toString(),
                    commandId = item.command.id,
                    parameterName = item.parameter.name,
                    labelOverride = null,
                    control = item.control,
                    helperText = item.parameter.description.takeIf { it.isNotBlank() }
                )
                spec.ensureSectionById(AdjustmentsSectionId).appendEntry(AdjustmentsSectionId, entry)
            } else {
                spec
            }
        }

        TelemetrySectionId -> {
            if (item is BuilderLibraryItem.Telemetry) {
                val entry = RemoteLayoutEntry.TelemetryEntry(
                    id = UUID.randomUUID().toString(),
                    sourceCommandId = item.command.id,
                    titleOverride = null,
                    display = RemoteLayoutEntry.TelemetryDisplay.COUNTER,
                    unit = null
                )
                spec.ensureSectionById(TelemetrySectionId).appendEntry(TelemetrySectionId, entry)
            } else {
                spec
            }
        }

        else -> spec
    }
}

private fun handleCanvasDrop(
    targetId: String,
    data: BuilderDragData.CanvasItem,
    spec: RemoteLayoutSpec
): RemoteLayoutSpec {
    if (targetId !in setOf(QuickActionsSectionId, AdjustmentsSectionId, TelemetrySectionId)) return spec

    val withoutOriginal = spec.updateSectionEntries(data.sectionId) { entries ->
        entries.filterIndexed { idx, _ -> idx != data.index }
    }

    val entry = data.entry
    return withoutOriginal.ensureSectionById(targetId).appendEntry(targetId, entry)
}

private fun RemoteLayoutEntry.displayTitle(commandName: String): String {
    return when (this) {
        is RemoteLayoutEntry.QuickActionEntry -> labelOverride ?: commandName
        is RemoteLayoutEntry.AdjustmentEntry -> labelOverride ?: parameterName
        is RemoteLayoutEntry.TelemetryEntry -> titleOverride ?: commandName
    }
}

private fun RemoteLayoutEntry.subtitle(commandName: String): String {
    return when (this) {
        is RemoteLayoutEntry.QuickActionEntry -> commandName
        is RemoteLayoutEntry.AdjustmentEntry -> "$commandName • $parameterName"
        is RemoteLayoutEntry.TelemetryEntry -> commandName
    }
}

private fun RemoteLayoutSpec.ensureSectionById(id: String): RemoteLayoutSpec {
    if (sections.any { it.id == id }) return this

    val newSection = when (id) {
        QuickActionsSectionId -> RemoteLayoutSpec.SectionSpec(
            id = QuickActionsSectionId,
            type = RemoteLayoutSpec.SectionType.QUICK_ACTIONS,
            title = "Quick Actions"
        )
        AdjustmentsSectionId -> RemoteLayoutSpec.SectionSpec(
            id = AdjustmentsSectionId,
            type = RemoteLayoutSpec.SectionType.ADJUSTMENTS,
            title = "Adjustments"
        )
        TelemetrySectionId -> RemoteLayoutSpec.SectionSpec(
            id = TelemetrySectionId,
            type = RemoteLayoutSpec.SectionType.TELEMETRY,
            title = "Telemetry"
        )
        else -> null
    } ?: return this

    return copy(sections = sections + newSection)
}

private fun RemoteLayoutSpec.ensureDefaultSections(): RemoteLayoutSpec {
    return ensureSectionById(QuickActionsSectionId)
        .ensureSectionById(AdjustmentsSectionId)
        .ensureSectionById(TelemetrySectionId)
}

private fun RemoteLayoutSpec.appendEntry(sectionId: String, entry: RemoteLayoutEntry): RemoteLayoutSpec {
    return updateSectionEntries(sectionId) { entries ->
        entries + entry
    }
}

private fun RemoteLayoutSpec.updateSectionEntries(
    sectionId: String,
    transformer: (List<RemoteLayoutEntry>) -> List<RemoteLayoutEntry>
): RemoteLayoutSpec {
    val updatedSections = sections.map { section ->
        if (section.id == sectionId) {
            section.copy(entries = transformer(section.entries))
        } else {
            section
        }
    }
    return copy(sections = updatedSections)
}

private fun List<RemoteLayoutEntry>.move(index: Int, direction: Int): List<RemoteLayoutEntry> {
    if (index !in indices) return this
    val targetIndex = (index + direction).coerceIn(0, size - 1)
    if (targetIndex == index) return this
    val mutable = toMutableList()
    val item = mutable.removeAt(index)
    mutable.add(targetIndex, item)
    return mutable.toList()
}

private data class BuilderLibrary(
    val quickActions: List<BuilderLibraryItem.QuickAction>,
    val adjustments: List<BuilderLibraryItem.Adjustment>,
    val telemetry: List<BuilderLibraryItem.Telemetry>
)

private sealed class BuilderLibraryItem(open val displayName: String) {
    data class QuickAction(val command: Command) : BuilderLibraryItem(command.name)
    data class Adjustment(val command: Command, val parameter: CommandParameter, val control: RemoteLayoutEntry.AdjustmentControl) :
        BuilderLibraryItem("${command.name} • ${parameter.name}")

    data class Telemetry(val command: Command) : BuilderLibraryItem(command.name)
}

private sealed class BuilderDragData {
    data class LibraryItem(val item: BuilderLibraryItem) : BuilderDragData()
    data class CanvasItem(
        val sectionId: String,
        val index: Int,
        val entry: RemoteLayoutEntry
    ) : BuilderDragData() {
        val displayName: String =
            when (entry) {
                is RemoteLayoutEntry.QuickActionEntry -> entry.labelOverride ?: entry.commandId
                is RemoteLayoutEntry.AdjustmentEntry -> entry.labelOverride ?: entry.parameterName
                is RemoteLayoutEntry.TelemetryEntry -> entry.titleOverride ?: entry.sourceCommandId
            }
    }
}

private fun buildLibrary(commands: List<Command>): BuilderLibrary {
    val quickActions = commands
        .filter { it.parameters.isEmpty() }
        .map { BuilderLibraryItem.QuickAction(it) }

    val adjustments = commands.flatMap { command ->
        command.parameters.mapNotNull { parameter ->
            when (parameter.type) {
                ParameterType.SLIDER -> BuilderLibraryItem.Adjustment(
                    command = command,
                    parameter = parameter,
                    control = RemoteLayoutEntry.AdjustmentControl.SLIDER
                )
                ParameterType.TOGGLE -> BuilderLibraryItem.Adjustment(
                    command = command,
                    parameter = parameter,
                    control = RemoteLayoutEntry.AdjustmentControl.TOGGLE
                )
                ParameterType.TEXT,
                ParameterType.DROPDOWN,
                ParameterType.UNSPECIFIED -> null
            }
        }
    }

    val telemetry = commands.map { BuilderLibraryItem.Telemetry(it) }

    return BuilderLibrary(
        quickActions = quickActions,
        adjustments = adjustments,
        telemetry = telemetry
    )
}

package com.handcontrol.feature.commands.shell

import android.view.inputmethod.InputMethodManager
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Cancel
import androidx.compose.material.icons.filled.Keyboard
import androidx.compose.material3.AssistChip
import androidx.compose.material3.AssistChipDefaults
import androidx.compose.material3.Button
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Card
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalLifecycleOwner
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.hilt.navigation.compose.hiltViewModel
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import com.handcontrol.data.commands.Command
import com.handcontrol.data.commands.ParameterInputState
import com.handcontrol.ui.components.parameters.ParameterInput
import com.termux.terminal.TerminalSession
import com.termux.terminal.TerminalSessionClient
import com.termux.view.TerminalView
import com.termux.view.TerminalViewClient
import timber.log.Timber
import java.text.DateFormat
import java.util.Date

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ShellSessionScreen(
    serverId: String,
    commandId: String,
    onNavigateBack: () -> Unit,
    modifier: Modifier = Modifier,
    viewModel: ShellSessionViewModel = hiltViewModel()
) {
    val uiState by viewModel.uiState.collectAsStateWithLifecycle()
    val snackbarHostState = remember { SnackbarHostState() }
    LaunchedEffect(serverId, commandId) {
        viewModel.load(serverId, commandId)
    }

    LaunchedEffect(Unit) {
        viewModel.toastMessages.collect { message ->
            snackbarHostState.showSnackbar(message)
        }
    }

    val lifecycleOwner = LocalLifecycleOwner.current
    DisposableEffect(lifecycleOwner, viewModel) {
        val observer = LifecycleEventObserver { _, event ->
            if (event == Lifecycle.Event.ON_STOP) {
                viewModel.detachSession()
            }
        }
        lifecycleOwner.lifecycle.addObserver(observer)
        onDispose {
            lifecycleOwner.lifecycle.removeObserver(observer)
            viewModel.detachSession()
        }
    }

    val errorMessage = uiState.errorMessage

    Scaffold(
        modifier = modifier,
        snackbarHost = { SnackbarHost(snackbarHostState) },
        topBar = {
            TopAppBar(
                title = {
                    Text(
                        text = uiState.command?.name ?: "Interactive Shell",
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis
                    )
                },
                navigationIcon = {
                    IconButton(
                        onClick = {
                            viewModel.detachSession()
                            onNavigateBack()
                        }
                    ) {
                        Icon(
                            imageVector = Icons.AutoMirrored.Filled.ArrowBack,
                            contentDescription = "Back"
                        )
                    }
                },
                actions = {
                    val terminalActive = uiState.connectionState is ShellConnectionState.Active
                    if (terminalActive || uiState.connectionState is ShellConnectionState.Connecting) {
                        if (terminalActive) {
                            IconButton(onClick = viewModel::toggleKeyboard) {
                                Icon(
                                    imageVector = Icons.Filled.Keyboard,
                                    contentDescription = "Toggle keyboard"
                                )
                            }
                        }
                        IconButton(onClick = { viewModel.closeSession("Client closed") }) {
                            Icon(
                                imageVector = Icons.Filled.Cancel,
                                contentDescription = "Close session"
                            )
                        }
                    }
                }
            )
        }
    ) { paddingValues ->
        when {
            uiState.isLoading -> {
                LoadingShellContent(Modifier.padding(paddingValues))
            }
            errorMessage != null && uiState.command == null -> {
                ErrorShellContent(
                    message = errorMessage,
                    onRetry = { viewModel.load(serverId, commandId) },
                    modifier = Modifier.padding(paddingValues)
                )
            }
            else -> {
                ShellSessionContent(
                    uiState = uiState,
                    onUpdateParameter = viewModel::updateParameter,
                    onRefreshDefault = viewModel::refreshDefault,
                    onResumeSession = viewModel::resumeSession,
                    onStartSession = viewModel::startSession,
                    onSendCtrlC = viewModel::sendCtrlC,
                    onRegisterBridge = viewModel::registerTerminalBridge,
                    onToggleKeyboard = viewModel::toggleKeyboard,
                    onCloseSession = viewModel::closeSession,
                    modifier = Modifier.padding(paddingValues)
                )
            }
        }
    }
}

@Composable
private fun ShellSessionContent(
    uiState: ShellSessionUiState,
    onUpdateParameter: (String, String) -> Unit,
    onRefreshDefault: (String) -> Unit,
    onResumeSession: (ResumeCandidate) -> Unit,
    onStartSession: () -> Unit,
    onSendCtrlC: () -> Unit,
    onRegisterBridge: (ComposeTerminalBridgeHandle) -> Unit,
    onToggleKeyboard: () -> Unit,
    onCloseSession: (String?) -> Unit,
    modifier: Modifier = Modifier
) {
    Column(
        modifier = modifier
            .fillMaxSize()
            .padding(horizontal = 16.dp, vertical = 12.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp)
    ) {
        uiState.command?.let { command ->
            Text(
                text = command.description,
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant
            )

            val showStartControls =
                uiState.connectionState !is ShellConnectionState.Active &&
                    uiState.connectionState !is ShellConnectionState.Connecting

            if (showStartControls) {
                if (uiState.resumeCandidates.isNotEmpty()) {
                    ResumeSessionSection(
                        candidates = uiState.resumeCandidates,
                        onResumeSession = onResumeSession,
                        modifier = Modifier.fillMaxWidth()
                    )

                    if (command.parameters.isNotEmpty()) {
                        Spacer(modifier = Modifier.height(8.dp))
                    }
                }

                if (command.parameters.isNotEmpty()) {
                    ParameterSection(
                        command = command,
                        parameterStates = uiState.parameterStates,
                        dynamicLoading = uiState.dynamicLoading,
                        onUpdateParameter = onUpdateParameter,
                        onRefreshDefault = onRefreshDefault
                    )
                }

                val startLabel = when (uiState.connectionState) {
                    is ShellConnectionState.Completed -> "Restart Session"
                    is ShellConnectionState.Failed -> "Retry Session"
                    is ShellConnectionState.Closed -> "Start Session"
                    else -> "Start Session"
                }

                Button(
                    onClick = onStartSession,
                    enabled = uiState.isFormValid,
                    modifier = Modifier.fillMaxWidth()
                ) {
                    Text(startLabel)
                }
            }
        }

        ConnectionStatus(uiState.connectionState)

        TerminalPane(
            terminalSession = uiState.terminalSession,
            connectionState = uiState.connectionState,
            statusMessage = uiState.statusMessage,
            onBridgeReady = onRegisterBridge,
            modifier = Modifier
                .weight(1f)
                .fillMaxWidth()
                .background(
                    color = MaterialTheme.colorScheme.surfaceVariant,
                    shape = RoundedCornerShape(12.dp)
                )
        )

        when (uiState.connectionState) {
            is ShellConnectionState.Active -> {
                TerminalShortcutRow(
                    onSendCtrlC = onSendCtrlC,
                    onToggleKeyboard = onToggleKeyboard,
                    onCloseSession = onCloseSession
                )
            }
            is ShellConnectionState.Connecting -> {
                LinearProgressIndicator(modifier = Modifier.fillMaxWidth())
            }
            is ShellConnectionState.Completed -> {
                Column(
                    modifier = Modifier.fillMaxWidth(),
                    verticalArrangement = Arrangement.spacedBy(8.dp)
                ) {
                    uiState.statusMessage?.takeIf { it.isNotBlank() }?.let { message ->
                        Text(
                            text = message,
                            style = MaterialTheme.typography.bodyMedium
                        )
                    }
                    Button(
                        onClick = onStartSession,
                        modifier = Modifier.fillMaxWidth()
                    ) {
                        Text("Restart Session")
                    }
                }
            }
            is ShellConnectionState.Failed -> {
                Column(
                    modifier = Modifier.fillMaxWidth(),
                    verticalArrangement = Arrangement.spacedBy(8.dp)
                ) {
                    Text(
                        text = uiState.connectionState.message,
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.error
                    )
                    Button(
                        onClick = onStartSession,
                        modifier = Modifier.fillMaxWidth()
                    ) {
                        Text("Retry")
                    }
                }
            }
            is ShellConnectionState.Closed -> {
                Text(
                    text = uiState.statusMessage ?: "Session closed",
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )
            }
            ShellConnectionState.NotStarted -> Unit
        }
    }
}

@Composable
private fun ParameterSection(
    command: Command,
    parameterStates: Map<String, ParameterInputState>,
    dynamicLoading: Map<String, Boolean>,
    onUpdateParameter: (String, String) -> Unit,
    onRefreshDefault: (String) -> Unit
) {
    Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
        command.parameters.forEach { param ->
            val state = parameterStates[param.name] ?: return@forEach
            ParameterInput(
                parameter = state.parameter,
                inputState = state,
                onValueChange = { value -> onUpdateParameter(param.name, value) },
                onRefreshDefault = if (param.defaultValueCommand != null) {
                    { onRefreshDefault(param.name) }
                } else null,
                isLoadingDefault = dynamicLoading[param.name] == true
            )
        }
    }
}

@Composable
private fun TerminalPane(
    terminalSession: TerminalSession?,
    connectionState: ShellConnectionState,
    statusMessage: String?,
    onBridgeReady: (ComposeTerminalBridgeHandle) -> Unit,
    modifier: Modifier = Modifier
) {
    Box(
        modifier = modifier
            .padding(12.dp),
        contentAlignment = Alignment.Center
    ) {
        if (terminalSession != null) {
            TerminalSurface(
                session = terminalSession,
                onBridgeReady = onBridgeReady,
                modifier = Modifier.fillMaxSize()
            )
        } else {
            TerminalPlaceholder(connectionState)
        }

        statusMessage?.takeIf { it.isNotBlank() }?.let { message ->
            Box(
                modifier = Modifier
                    .align(Alignment.BottomStart)
                    .background(
                        color = MaterialTheme.colorScheme.surfaceVariant.copy(alpha = 0.86f),
                        shape = RoundedCornerShape(8.dp)
                    )
                    .padding(8.dp)
            ) {
                Text(
                    text = message,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = 2,
                    overflow = TextOverflow.Ellipsis
                )
            }
        }
    }
}

@Composable
private fun TerminalSurface(
    session: TerminalSession,
    onBridgeReady: (ComposeTerminalBridgeHandle) -> Unit,
    modifier: Modifier = Modifier
) {
    val context = LocalContext.current
    val bridge = remember { ComposeTerminalBridge(context) }

    LaunchedEffect(bridge) {
        onBridgeReady(bridge)
    }

    AndroidView(
        modifier = modifier.clip(RoundedCornerShape(10.dp)),
        factory = { ctx ->
            TerminalView(ctx, null).apply {
                isFocusable = true
                isFocusableInTouchMode = true
                setTerminalViewClient(bridge)
                bridge.bindView(this)
                bridge.bindSession(session)
                attachSession(session)
                requestFocus()
            }
        },
        update = { view ->
            bridge.bindView(view)
            bridge.bindSession(session)
            if (view.attachSession(session)) {
                session.updateTerminalSessionClient(bridge)
            }
        }
    )

    LaunchedEffect(session.mHandle) {
        bridge.focusAndShowKeyboard()
    }
}

@Composable
private fun TerminalPlaceholder(connectionState: ShellConnectionState) {
    val message = when (connectionState) {
        ShellConnectionState.NotStarted,
        is ShellConnectionState.Closed,
        is ShellConnectionState.Completed -> "Start the session to open a terminal."
        is ShellConnectionState.Connecting -> "Starting remote terminal..."
        is ShellConnectionState.Failed -> "Session unavailable."
        is ShellConnectionState.Active -> "Preparing terminal view…"
    }

    Column(
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(8.dp)
    ) {
        Icon(
            imageVector = Icons.Filled.Keyboard,
            contentDescription = null,
            modifier = Modifier.size(32.dp),
            tint = MaterialTheme.colorScheme.outline
        )
        Text(
            text = message,
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            textAlign = TextAlign.Center
        )
    }
}

@Composable
private fun TerminalShortcutRow(
    onSendCtrlC: () -> Unit,
    onToggleKeyboard: () -> Unit,
    onCloseSession: (String?) -> Unit,
    modifier: Modifier = Modifier
) {
    Row(
        modifier = modifier.fillMaxWidth(),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp)
    ) {
        OutlinedButton(onClick = onSendCtrlC) {
            Text("Ctrl+C")
        }
        OutlinedButton(onClick = onToggleKeyboard) {
            Text("Toggle Keyboard")
        }
        OutlinedButton(onClick = { onCloseSession("Client closed") }) {
            Text("Close Session")
        }
        Spacer(modifier = Modifier.weight(1f))
    }
}

private class ComposeTerminalBridge(
    private val context: android.content.Context
) : TerminalSessionClient, TerminalViewClient, ComposeTerminalBridgeHandle {

    private val clipboard =
        context.getSystemService(android.content.Context.CLIPBOARD_SERVICE) as android.content.ClipboardManager
    private val inputMethodManager =
        context.getSystemService(android.content.Context.INPUT_METHOD_SERVICE) as InputMethodManager
    private var terminalView: TerminalView? = null
    private var boundSession: TerminalSession? = null
    private var textSizeDp = 14
    private var keyboardVisible = false

    fun bindView(view: TerminalView) {
        terminalView = view
        view.setTextSize(textSizeDp)
        keyboardVisible = inputMethodManager.isActive(view)
    }

    fun bindSession(session: TerminalSession) {
        boundSession = session
        session.updateTerminalSessionClient(this)
    }

    fun focusAndShowKeyboard() {
        val view = terminalView ?: return
        view.requestFocus()
        view.post {
            inputMethodManager.showSoftInput(view, InputMethodManager.SHOW_IMPLICIT)
            keyboardVisible = true
        }
    }

    override fun toggleKeyboard() {
        val view = terminalView ?: return
        if (keyboardVisible) {
            hideKeyboard()
        } else {
            focusAndShowKeyboard()
        }
    }

    override fun hideKeyboard() {
        val view = terminalView ?: return
        inputMethodManager.hideSoftInputFromWindow(view.windowToken, 0)
        keyboardVisible = false
    }

    override fun onTextChanged(changedSession: TerminalSession) {
        terminalView?.onScreenUpdated()
        terminalView?.postInvalidateOnAnimation()
    }

    override fun onTitleChanged(changedSession: TerminalSession) = Unit

    override fun onSessionFinished(finishedSession: TerminalSession) {
        Timber.i("Terminal session finished")
    }

    override fun onCopyTextToClipboard(session: TerminalSession, text: String) {
        clipboard.setPrimaryClip(android.content.ClipData.newPlainText("terminal", text))
    }

    override fun onPasteTextFromClipboard(session: TerminalSession) {
        val paste = clipboard.primaryClip?.getItemAt(0)?.coerceToText(context)?.toString()
        if (!paste.isNullOrEmpty()) {
            session.write(paste)
        }
    }

    override fun onBell(session: TerminalSession) {
        terminalView?.performHapticFeedback(android.view.HapticFeedbackConstants.VIRTUAL_KEY)
    }

    override fun onColorsChanged(session: TerminalSession) {
        terminalView?.postInvalidateOnAnimation()
    }

    override fun onTerminalCursorStateChange(state: Boolean) {
        terminalView?.postInvalidateOnAnimation()
    }

    override fun getTerminalCursorStyle(): Int? = null

    override fun logError(tag: String, message: String) = Timber.e("$tag: $message")
    override fun logWarn(tag: String, message: String) = Timber.w("$tag: $message")
    override fun logInfo(tag: String, message: String) = Timber.i("$tag: $message")
    override fun logDebug(tag: String, message: String) = Timber.d("$tag: $message")
    override fun logVerbose(tag: String, message: String) = Timber.v("$tag: $message")
    override fun logStackTraceWithMessage(tag: String, message: String, e: Exception) =
        Timber.e(e, "$tag: $message")
    override fun logStackTrace(tag: String, e: Exception) = Timber.e(e, tag)

    override fun onScale(scale: Float): Float {
        if (scale < 0.9f || scale > 1.1f) {
            textSizeDp = (textSizeDp + if (scale > 1f) 1 else -1).coerceIn(8, 24)
            terminalView?.setTextSize(textSizeDp)
            return 1f
        }
        return scale
    }

    override fun onSingleTapUp(e: android.view.MotionEvent) {
        focusAndShowKeyboard()
    }

    override fun shouldBackButtonBeMappedToEscape(): Boolean = false

    override fun shouldEnforceCharBasedInput(): Boolean = true

    override fun shouldUseCtrlSpaceWorkaround(): Boolean = false

    override fun isTerminalViewSelected(): Boolean = terminalView?.isFocused == true

    override fun copyModeChanged(copyMode: Boolean) = Unit

    override fun onKeyDown(
        keyCode: Int,
        e: android.view.KeyEvent,
        session: TerminalSession
    ): Boolean = false

    override fun onKeyUp(keyCode: Int, e: android.view.KeyEvent): Boolean = false

    override fun onLongPress(event: android.view.MotionEvent): Boolean = false

    override fun readControlKey(): Boolean = false

    override fun readAltKey(): Boolean = false

    override fun readShiftKey(): Boolean = false

    override fun readFnKey(): Boolean = false

    override fun onCodePoint(codePoint: Int, ctrlDown: Boolean, session: TerminalSession): Boolean =
        false

    override fun onEmulatorSet() {
        Timber.d("Terminal emulator attached")
        focusAndShowKeyboard()
    }
}

@Composable
private fun ResumeSessionSection(
    candidates: List<ResumeCandidate>,
    onResumeSession: (ResumeCandidate) -> Unit,
    modifier: Modifier = Modifier
) {
    Column(
        modifier = modifier,
        verticalArrangement = Arrangement.spacedBy(8.dp)
    ) {
        Text(
            text = "Resume an existing session",
            style = MaterialTheme.typography.titleMedium
        )

        Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
            candidates.forEach { candidate ->
                Card(
                    modifier = Modifier.fillMaxWidth(),
                    shape = RoundedCornerShape(12.dp)
                ) {
                    Column(
                        modifier = Modifier
                            .fillMaxWidth()
                            .padding(12.dp),
                        verticalArrangement = Arrangement.spacedBy(6.dp)
                    ) {
                        Text(
                            text = candidate.capabilityName,
                            style = MaterialTheme.typography.titleMedium
                        )

                        val shortId = remember(candidate.sessionId) {
                            if (candidate.sessionId.length > 12) {
                                candidate.sessionId.take(8) + "…"
                            } else {
                                candidate.sessionId
                            }
                        }

                        Text(
                            text = "Session ID: $shortId",
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant
                        )

                        val lastActivity = candidate.lastActivityMs.takeIf { it > 0 }
                        if (lastActivity != null) {
                            val lastActivityText = remember(lastActivity) {
                                DateFormat.getDateTimeInstance(DateFormat.SHORT, DateFormat.SHORT)
                                    .format(Date(lastActivity))
                            }
                            Text(
                                text = "Last activity: $lastActivityText",
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant
                            )
                        }

                        if (candidate.bufferLength > 0) {
                            Text(
                                text = "Buffered frames: ${candidate.bufferLength}",
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant
                            )
                        }

                        Button(
                            onClick = { onResumeSession(candidate) },
                            modifier = Modifier.fillMaxWidth()
                        ) {
                            Text("Resume session")
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun ConnectionStatus(state: ShellConnectionState) {
    val (label, color) = when (state) {
        is ShellConnectionState.NotStarted -> "Session not started" to MaterialTheme.colorScheme.outline
        is ShellConnectionState.Connecting -> "Connecting…" to MaterialTheme.colorScheme.primary
        is ShellConnectionState.Active -> {
            val message = state.readyMessage?.takeIf { it.isNotBlank() } ?: "Connected"
            message to MaterialTheme.colorScheme.primary
        }
        is ShellConnectionState.Completed -> {
            val message = state.message?.takeIf { it.isNotBlank() } ?: if (state.exitCode == 0) {
                "Exited successfully"
            } else {
                "Exited (${state.exitCode})"
            }
            message to MaterialTheme.colorScheme.tertiary
        }
        is ShellConnectionState.Failed -> state.message to MaterialTheme.colorScheme.error
        is ShellConnectionState.Closed -> {
            val reason = state.reason?.takeIf { it.isNotBlank() }
            val message = reason?.let { "Closed: $it" } ?: "Closed"
            message to MaterialTheme.colorScheme.outline
        }
    }

    AssistChip(
        onClick = {},
        label = { Text(label) },
        colors = AssistChipDefaults.assistChipColors(
            containerColor = color.copy(alpha = 0.16f),
            labelColor = color
        ),
        leadingIcon = {
            Icon(
                imageVector = Icons.Filled.Keyboard,
                contentDescription = null,
                modifier = Modifier.size(16.dp),
                tint = color
            )
        }
    )
}

@Composable
private fun LoadingShellContent(modifier: Modifier = Modifier) {
    Box(
        modifier = modifier
            .fillMaxSize(),
        contentAlignment = Alignment.Center
    ) {
        Column(horizontalAlignment = Alignment.CenterHorizontally) {
            LinearProgressIndicator(modifier = Modifier.fillMaxWidth(0.6f))
            Spacer(modifier = Modifier.height(16.dp))
            Text("Loading shell capability…")
        }
    }
}

@Composable
private fun ErrorShellContent(
    message: String,
    onRetry: () -> Unit,
    modifier: Modifier = Modifier
) {
    Box(
        modifier = modifier.fillMaxSize(),
        contentAlignment = Alignment.Center
    ) {
        Column(
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(12.dp)
        ) {
            Text(
                text = message,
                style = MaterialTheme.typography.bodyLarge,
                color = MaterialTheme.colorScheme.error,
                textAlign = androidx.compose.ui.text.style.TextAlign.Center
            )
            Button(onClick = onRetry) {
                Text("Retry")
            }
        }
    }
}

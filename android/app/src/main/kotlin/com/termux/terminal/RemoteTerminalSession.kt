package com.termux.terminal

import com.handcontrol.data.commands.ShellSession
import com.handcontrol.data.commands.ShellSessionEvent
import com.handcontrol.data.commands.TerminalSize
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import timber.log.Timber
import kotlinx.coroutines.cancelAndJoin
import java.util.ArrayDeque

/**
 * Bridges Termux's [TerminalSession] to a remote shell session streamed over gRPC.
 *
 * The Termux classes expect to own a local subprocess; this subclass replaces the
 * subprocess with a [ShellSession] that delivers PTY bytes through callbacks.
 */
class RemoteTerminalSession(
    private val scope: CoroutineScope,
    private val shellSession: ShellSession,
    private val transcriptRows: Int? = null,
    client: TerminalSessionClient,
    private val onEvent: (ShellSessionEvent) -> Unit
) : TerminalSession(
    /* shellPath = */ "",
    /* cwd = */ null,
    /* args = */ emptyArray(),
    /* env = */ emptyArray(),
    /* transcriptRows = */ transcriptRows,
    client
) {

    private val sessionJob: Job
    private val ioScope = CoroutineScope(scope.coroutineContext + SupervisorJob())

    @Volatile
    private var running = true

    @Volatile
    private var finishedNotified = false

    private val pendingOutput = ArrayDeque<ByteArray>()

    init {
        sessionJob = startConsumingEvents()
    }

    private fun startConsumingEvents(): Job {
        return scope.launch {
            shellSession.events.collect { event ->
                withContext(Dispatchers.Main.immediate) {
                    when (event) {
                        is ShellSessionEvent.Output -> handleOutput(event)
                        is ShellSessionEvent.Exit -> handleSessionTerminated(event)
                        is ShellSessionEvent.Closed -> handleSessionClosed(event)
                        else -> onEvent(event)
                    }
                }
            }
        }
    }

    private fun handleOutput(event: ShellSessionEvent.Output) {
        if (!running) return

        // Feed raw bytes back into the terminal emulator.
        if (event.data.isNotEmpty()) {
            val emulator = mEmulator
            if (emulator != null) {
                emulator.append(event.data, event.data.size)
                notifyScreenUpdate()
            } else {
                pendingOutput += event.data.copyOf()
            }
        }

        onEvent(event)
    }

    private fun handleSessionTerminated(event: ShellSessionEvent.Exit) {
        running = false
        onEvent(event)
        notifyFinished()
    }

    private fun handleSessionClosed(event: ShellSessionEvent.Closed) {
        running = false
        onEvent(event)
        notifyFinished()
    }

    private fun notifyFinished() {
        if (!finishedNotified) {
            finishedNotified = true
            mClient.onSessionFinished(this)
        }
    }

    override fun initializeEmulator(columns: Int, rows: Int) {
        mEmulator = TerminalEmulator(this, columns, rows, transcriptRows, mClient)
        flushPendingOutput()
    }

    override fun updateSize(columns: Int, rows: Int) {
        if (mEmulator == null) {
            initializeEmulator(columns, rows)
        } else {
            mEmulator.resize(columns, rows)
        }

        ioScope.launch {
            try {
                shellSession.resize(TerminalSize(columns, rows))
            } catch (e: Exception) {
                Timber.w(e, "Failed to propagate terminal resize")
            }
        }
    }

    override fun write(data: ByteArray, offset: Int, count: Int) {
        if (!running || count <= 0) return
        val payload = data.copyOfRange(offset, offset + count)
        ioScope.launch {
            try {
                shellSession.writeInput(payload)
            } catch (e: Exception) {
                Timber.e(e, "Failed to forward terminal input")
                onEvent(ShellSessionEvent.Error(e.message ?: "Failed to send input"))
            }
        }
    }

    override fun finishIfRunning() {
        if (!running) return
        running = false
        ioScope.launch {
            try {
                shellSession.close("Client closed")
            } catch (e: Exception) {
                Timber.w(e, "Failed to close remote shell session")
            } finally {
                notifyFinished()
            }
        }
    }

    suspend fun awaitCompletion() {
        sessionJob.join()
    }

    suspend fun dispose(reason: String? = null) {
        running = false
        try {
            shellSession.close(reason)
        } catch (e: Exception) {
            Timber.w(e, "Failed to close session during dispose")
        } finally {
            ioScope.coroutineContext[Job]?.cancel()
            sessionJob.cancelAndJoin()
            notifyFinished()
        }
    }

    private fun flushPendingOutput() {
        val emulator = mEmulator ?: return
        while (pendingOutput.isNotEmpty()) {
            val chunk = pendingOutput.removeFirst()
            emulator.append(chunk, chunk.size)
        }
        notifyScreenUpdate()
    }
}

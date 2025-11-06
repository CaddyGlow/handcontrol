package com.handcontrol.data.commands

import com.handcontrol.core.network.ConnectionResult
import com.handcontrol.core.network.RelayConnectionException
import com.handcontrol.core.network.ServerConnectionManager
import com.handcontrol.core.security.VerificationCodeGenerator
import com.handcontrol.data.database.EnrolledServerEntity
import com.handcontrol.data.database.EnrolledServerRepository
import com.google.protobuf.ByteString
import com.handcontrol.grpc.CapabilityKind as ProtoCapabilityKind
import com.handcontrol.grpc.CapabilityParameterType as ProtoCapabilityParameterType
import com.handcontrol.grpc.ListCapabilitiesRequest
import com.handcontrol.grpc.ListSessionsRequest
import com.handcontrol.grpc.RemoteControlGrpcKt
import com.handcontrol.grpc.ServerInfoRequest
import com.handcontrol.grpc.SessionClientMessage
import com.handcontrol.grpc.SessionClose
import com.handcontrol.grpc.SessionHeartbeat
import com.handcontrol.grpc.SessionInput
import com.handcontrol.grpc.SessionMode as ProtoSessionMode
import com.handcontrol.grpc.SessionOpen
import com.handcontrol.grpc.SessionResize
import com.handcontrol.grpc.SessionResume
import io.grpc.Status
import io.grpc.StatusException
import io.grpc.StatusRuntimeException
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicReference
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.catch
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.receiveAsFlow
import kotlinx.coroutines.flow.onCompletion
import kotlinx.coroutines.flow.transform
import kotlinx.coroutines.flow.callbackFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.delay
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.Job
import kotlinx.coroutines.isActive
import kotlinx.coroutines.flow.flow
import kotlinx.coroutines.withContext
import kotlinx.coroutines.channels.awaitClose
import timber.log.Timber
import javax.inject.Inject
import javax.inject.Singleton
import kotlin.text.Charsets
import kotlin.math.max

@Singleton
class GrpcCommandRepository @Inject constructor(
    private val connectionManager: ServerConnectionManager,
    private val enrolledServerRepository: EnrolledServerRepository
) : CommandRepository {

    override suspend fun getServerInfo(serverId: String): Result<ServerInfo> {
        return try {
            val server = enrolledServerRepository.getServerById(serverId)
                ?: return Result.failure(Exception("Server not found: $serverId"))

            Timber.i("Connecting to server ${server.serverName} for server info")
            val connectionResult = connectionManager.connect(server)
            val result = try {
                val channel = connectionResult.channel

                Timber.d("Connected via ${connectionResult.mode} mode")
                val stub = RemoteControlGrpcKt.RemoteControlCoroutineStub(channel)

                val request = ServerInfoRequest.newBuilder().build()
                val response = stub.getServerInfo(request)

                verifyServerCertificate(server, connectionResult)

                // Persist successful connection mode
                enrolledServerRepository.updateConnectionMode(serverId, connectionResult.mode)

                Result.success(
                    ServerInfo(
                        serverId = response.serverId,
                        hostname = response.hostname,
                        version = response.version,
                        os = response.os
                    )
                )
            } finally {
                connectionManager.disconnect(connectionResult)
            }

            result
        } catch (e: RelayConnectionException) {
            Timber.e(e, "Relay connection failed: ${e.shortMessage}")
            Result.failure(Exception(e.userMessage, e))
        } catch (e: StatusException) {
            Timber.e(e, "Failed to get server info")
            Result.failure(Exception(mapGrpcError(e.status)))
        } catch (e: SecurityException) {
            Timber.e(e, "Server certificate verification failed")
            Result.failure(e)
        } catch (e: Exception) {
            Timber.e(e, "Failed to get server info")
            Result.failure(e)
        }
    }

    override suspend fun listCommands(serverId: String): Result<List<Command>> {
        return try {
            val server = enrolledServerRepository.getServerById(serverId)
                ?: return Result.failure(Exception("Server not found: $serverId"))

            Timber.i("Connecting to server ${server.serverName} to list capabilities")
            val connectionResult = connectionManager.connect(server)
            val result = try {
                val channel = connectionResult.channel

                Timber.d("Connected via ${connectionResult.mode} mode")
                val stub = RemoteControlGrpcKt.RemoteControlCoroutineStub(channel)

                val request = ListCapabilitiesRequest.newBuilder().build()
                val response = stub.listCapabilities(request)

                verifyServerCertificate(server, connectionResult)

                // Persist successful connection mode
                enrolledServerRepository.updateConnectionMode(serverId, connectionResult.mode)

                val commands = response.capabilitiesList.map { protoCapability ->
                    val parameters = protoCapability.parametersList.map { protoParam ->
                        CommandParameter(
                            name = protoParam.name,
                            type = mapParameterType(protoParam.type),
                            description = protoParam.description,
                            min = if (protoParam.hasMin()) protoParam.min else null,
                            max = if (protoParam.hasMax()) protoParam.max else null,
                            defaultValue = if (protoParam.hasDefaultValue()) protoParam.defaultValue else null,
                            options = protoParam.optionsList,
                            validation = if (protoParam.hasValidation()) protoParam.validation else null,
                            labelOn = if (protoParam.hasLabelOn()) protoParam.labelOn else null,
                            labelOff = if (protoParam.hasLabelOff()) protoParam.labelOff else null,
                            defaultValueCommand = if (protoParam.hasDefaultValueCommand()) protoParam.defaultValueCommand else null,
                            defaultValuePattern = if (protoParam.hasDefaultValuePattern()) protoParam.defaultValuePattern else null
                        )
                    }

                    Command(
                        id = protoCapability.id,
                        name = protoCapability.name,
                        description = protoCapability.description,
                        icon = null,
                        tags = protoCapability.tagsList,
                        parameters = parameters,
                        requiresConfirmation = protoCapability.hasRequiresConfirmation() && protoCapability.requiresConfirmation,
                        showOutput = true,
                        privileged = protoCapability.hasPrivileged() && protoCapability.privileged,
                        kind = mapCapabilityKind(protoCapability.kind),
                        sessionMode = mapSessionMode(protoCapability.sessionMode)
                    )
                }
                Timber.i("Loaded ${commands.size} capabilities from server")
                Result.success(commands)
            } finally {
                connectionManager.disconnect(connectionResult)
            }

            result
        } catch (e: RelayConnectionException) {
            Timber.e(e, "Relay connection failed: ${e.shortMessage}")
            Result.failure(Exception(e.userMessage, e))
        } catch (e: StatusException) {
            Timber.e(e, "Failed to list capabilities")
            Result.failure(Exception(mapGrpcError(e.status)))
        } catch (e: SecurityException) {
            Timber.e(e, "Server certificate verification failed")
            Result.failure(e)
        } catch (e: Exception) {
            Timber.e(e, "Failed to list capabilities")
            Result.failure(e)
        }
    }

    override suspend fun executeCommand(
        serverId: String,
        commandId: String,
        parameters: Map<String, String>
    ): Flow<CommandExecutionResult> {
        val server = enrolledServerRepository.getServerById(serverId)
            ?: throw Exception("Server not found: $serverId")

        Timber.i("Connecting to server ${server.serverName} to execute capability: $commandId")
        val connectionResult = connectionManager.connect(server)
        val channel = connectionResult.channel

        Timber.d("Connected via ${connectionResult.mode} mode")
        val stub = RemoteControlGrpcKt.RemoteControlCoroutineStub(channel)

        val openMessageBuilder = SessionOpen.newBuilder()
            .setCapabilityId(commandId)
        openMessageBuilder.putAllParameters(parameters)

        val sessionOpen = openMessageBuilder.build()
        val clientOpenMessage = SessionClientMessage.newBuilder()
            .setOpen(sessionOpen)
            .build()

        Timber.i("Opening capability session for $commandId with parameters: $parameters")
        val sessionRequest = flow { emit(clientOpenMessage) }

        var sessionValidated = false

        return stub.openSession(sessionRequest)
            .transform { message ->
                when {
                    message.hasReady() -> {
                        val ready = message.ready
                        val sessionMode = mapSessionMode(ready.sessionMode)
                        Timber.d("Capability $commandId ready with session mode $sessionMode")
                        if (sessionMode != CommandSessionMode.ONE_SHOT) {
                            Timber.w("Capability $commandId requires unsupported session mode $sessionMode")
                            throw UnsupportedOperationException(
                                "Capability $commandId requires $sessionMode sessions which are not supported on Android yet"
                            )
                        }
                        sessionValidated = true
                    }
                    message.hasOutput() -> {
                        val output = message.output
                        val isStdErr = output.hasStderr() && output.stderr
                        val isBinary = output.hasBinary() && output.binary
                        val text = if (isBinary) {
                            "[binary data: ${output.data.size()} bytes]"
                        } else {
                            output.data.toStringUtf8()
                        }
                        Timber.d("Capability $commandId ${if (isStdErr) "stderr" else "stdout"}: $text")
                        emit(CommandExecutionResult.Output(text, isError = isStdErr))
                    }
                    message.hasExit() -> {
                        val exit = message.exit
                        Timber.i("Capability $commandId completed with exit code ${exit.exitCode}")
                        if (exit.hasMessage() && exit.message.isNotBlank()) {
                            emit(CommandExecutionResult.Output(exit.message, isError = exit.timedOut))
                        }
                        emit(CommandExecutionResult.ExitCode(exit.exitCode))
                    }
                    message.hasError() -> {
                        val error = message.error
                        Timber.e("Capability $commandId error: ${error.message}")
                        emit(CommandExecutionResult.Error(error.message))
                    }
                    message.hasHeartbeat() -> {
                        // Heartbeat ACK for realtime sessions (ignore for one-shot)
                    }
                    message.hasClosed() -> {
                        Timber.d("Capability $commandId session closed by server")
                    }
                    else -> {
                        Timber.w("Capability $commandId emitted unrecognised message: $message")
                    }
                }
            }
            .catch { e ->
                Timber.e(e, "Capability execution failed")
                when (e) {
                    is StatusException -> emit(CommandExecutionResult.Error(mapGrpcError(e.status)))
                    is SecurityException -> emit(CommandExecutionResult.Error(e.message ?: "Security verification failed"))
                    is UnsupportedOperationException -> emit(CommandExecutionResult.Error(e.message ?: "Capability requires unsupported session mode"))
                    else -> emit(CommandExecutionResult.Error(e.message ?: "Unknown error"))
                }
            }
            .onCompletion { cause ->
                // Disconnect channel when flow completes (success or error)
                Timber.d("Capability execution flow completed (cause: $cause, validated=$sessionValidated)")

                var securityFailure: SecurityException? = null
                try {
                    verifyServerCertificate(server, connectionResult)
                } catch (e: SecurityException) {
                    securityFailure = e
                } finally {
                    connectionManager.disconnect(connectionResult)
                }

                if (securityFailure == null && (cause == null || cause is StatusException)) {
                    enrolledServerRepository.updateConnectionMode(serverId, connectionResult.mode)
                }

                securityFailure?.let { throw it }
            }
    }

    override suspend fun listSessions(serverId: String): Result<List<ActiveCommandSession>> =
        withContext(Dispatchers.IO) {
            val server = enrolledServerRepository.getServerById(serverId)
                ?: return@withContext Result.failure(Exception("Server not found: $serverId"))

            val connectionResult = try {
                connectionManager.connect(server)
            } catch (e: Exception) {
                Timber.e(e, "Failed to connect for listing sessions")
                return@withContext Result.failure(e)
            }

            val stub = RemoteControlGrpcKt.RemoteControlCoroutineStub(connectionResult.channel)

            try {
                val response = stub.listSessions(ListSessionsRequest.newBuilder().build())
                try {
                    verifyServerCertificate(server, connectionResult)
                    enrolledServerRepository.updateConnectionMode(serverId, connectionResult.mode)
                } catch (e: SecurityException) {
                    Timber.e(e, "Certificate verification failed while listing sessions")
                    return@withContext Result.failure(e)
                }

                val sessions = response.sessionsList.map { info ->
                    ActiveCommandSession(
                        sessionId = info.sessionId,
                        capabilityId = info.capabilityId,
                        capabilityName = info.capabilityName,
                        sessionMode = mapSessionMode(info.sessionMode),
                        attached = info.attached,
                        ownerFingerprint = info.ownerFingerprint,
                        createdAtMs = info.createdAtMs,
                        lastActivityMs = info.lastActivityMs,
                        lastDetachedMs = if (info.hasLastDetachedMs()) info.lastDetachedMs else null,
                        resumeToken = info.resumeToken,
                        stdoutNextSequence = info.stdoutNextSequence,
                        stderrNextSequence = info.stderrNextSequence,
                        bufferLength = info.bufferLen
                    )
                }

                Result.success(sessions)
            } catch (e: StatusException) {
                Timber.e(e, "ListSessions RPC failed")
                Result.failure(Exception(mapGrpcError(e.status)))
            } catch (e: Exception) {
                Timber.e(e, "Failed to list sessions")
                Result.failure(e)
            } finally {
                connectionManager.disconnect(connectionResult)
            }
        }

    override suspend fun openShellSession(
        serverId: String,
        commandId: String,
        parameters: Map<String, String>,
        terminalSize: TerminalSize?,
        resumeSpec: ResumeSessionSpec?
    ): Result<ShellSession> = withContext(Dispatchers.IO) {
        val server = enrolledServerRepository.getServerById(serverId)
            ?: return@withContext Result.failure(Exception("Server not found: $serverId"))

        Timber.i("Opening realtime shell session to capability $commandId on ${server.serverName}")

        val writeHandler = AtomicReference<suspend (ByteArray) -> Unit> {
            throw IllegalStateException("Shell session is not ready yet")
        }
        val resizeHandler = AtomicReference<suspend (TerminalSize) -> Unit> { _ -> }
        val closeHandler = AtomicReference<suspend (String?) -> Unit> { _ -> }
        val lastTerminalSize = AtomicReference(terminalSize)
        val userClosed = AtomicBoolean(false)

        val events = callbackFlow<ShellSessionEvent> {
            val resumeState = CapabilityResumeState().apply {
                if (resumeSpec != null) {
                    sessionId = resumeSpec.sessionId
                    resumeToken = resumeSpec.resumeToken
                    lastStdoutSequence = resumeSpec.lastStdoutSequence
                    lastStderrSequence = resumeSpec.lastStderrSequence
                }
            }
            var resumeAttempts = 0
            var pendingReconnect = resumeSpec != null

            fun shouldAttemptResume(throwable: Throwable): Boolean {
                if (userClosed.get() || !resumeState.canResume()) return false
                val status = when (throwable) {
                    is StatusException -> throwable.status
                    is StatusRuntimeException -> throwable.status
                    else -> null
                }
                return when (status?.code) {
                    Status.Code.CANCELLED,
                    Status.Code.INVALID_ARGUMENT,
                    Status.Code.NOT_FOUND,
                    Status.Code.PERMISSION_DENIED,
                    Status.Code.UNIMPLEMENTED,
                    Status.Code.ALREADY_EXISTS,
                    Status.Code.DATA_LOSS -> false
                    Status.Code.UNAVAILABLE,
                    Status.Code.UNKNOWN,
                    Status.Code.DEADLINE_EXCEEDED,
                    Status.Code.INTERNAL,
                    Status.Code.ABORTED -> true
                    null -> throwable is java.io.IOException
                    else -> false
                }
            }

            while (isActive && !userClosed.get()) {
                val connectionResult = try {
                    connectionManager.connect(server)
                } catch (e: Exception) {
                    Timber.e(e, "Failed to connect for shell session")
                    trySend(ShellSessionEvent.Error(e.message ?: "Failed to connect to server"))
                    break
                }

                Timber.d("Connected via ${connectionResult.mode} mode for realtime session")

                val requestChannel = Channel<SessionClientMessage>(capacity = Channel.BUFFERED)
                val sessionIdDeferred = CompletableDeferred<String>()
                if (pendingReconnect && resumeState.sessionId != null) {
                    sessionIdDeferred.complete(resumeState.sessionId!!)
                }
                val certificateVerified = AtomicBoolean(false)
                val heartbeatIntervalMs = 15_000L
                var heartbeatJob: Job? = null
                var pendingResize = lastTerminalSize.get()
                var resumePlanned = false
                var failure: Throwable? = null

                fun scheduleHeartbeat() {
                    if (heartbeatJob != null) return
                    heartbeatJob = launch {
                        val sessionId = sessionIdDeferred.await()
                        while (isActive && !userClosed.get()) {
                            try {
                                delay(heartbeatIntervalMs)
                                val heartbeat = SessionClientMessage.newBuilder()
                                    .setSessionId(sessionId)
                                    .setHeartbeat(
                                        SessionHeartbeat.newBuilder()
                                            .setTimestampMs(System.currentTimeMillis())
                                            .build()
                                    )
                                    .build()
                                requestChannel.send(heartbeat)
                            } catch (e: Exception) {
                                Timber.v(e, "Heartbeat loop terminating for $commandId")
                                break
                            }
                        }
                    }
                }

                val stub = RemoteControlGrpcKt.RemoteControlCoroutineStub(connectionResult.channel)

                writeHandler.set { data ->
                    val sessionId = sessionIdDeferred.await()
                    val inputMessage = SessionClientMessage.newBuilder()
                        .setSessionId(sessionId)
                        .setInput(
                            SessionInput.newBuilder()
                                .setData(ByteString.copyFrom(data))
                                .build()
                        )
                        .build()
                    requestChannel.send(inputMessage)
                }

                resizeHandler.set { size ->
                    lastTerminalSize.set(size)
                    val sessionId = sessionIdDeferred.await()
                    val resizeMessage = SessionClientMessage.newBuilder()
                        .setSessionId(sessionId)
                        .setResize(
                            SessionResize.newBuilder()
                                .setCols(size.cols)
                                .setRows(size.rows)
                                .build()
                        )
                        .build()
                    requestChannel.send(resizeMessage)
                }

                closeHandler.set { reason ->
                    if (userClosed.compareAndSet(false, true)) {
                        Timber.d("Closing realtime shell session locally (reason=$reason)")
                    }
                    val sessionId = sessionIdDeferred.await()
                    val closeMessage = SessionClientMessage.newBuilder()
                        .setSessionId(sessionId)
                        .setClose(
                            SessionClose.newBuilder()
                                .apply { reason?.let { setReason(it) } }
                                .build()
                        )
                        .build()
                    try {
                        requestChannel.send(closeMessage)
                    } catch (e: Exception) {
                        Timber.w(e, "Failed to send close message for shell session")
                    } finally {
                        requestChannel.close()
                    }
                }

                val resuming = pendingReconnect && resumeState.canResume()
                val initialMessage = if (resuming) {
                    val resume = SessionResume.newBuilder()
                        .setResumeToken(resumeState.resumeToken)
                        .setLastOutputSequence(resumeState.lastStdoutSequence)
                        .setLastErrorSequence(resumeState.lastStderrSequence)
                        .build()
                    SessionClientMessage.newBuilder()
                        .setSessionId(resumeState.sessionId)
                        .setResume(resume)
                        .build()
                } else {
                    SessionClientMessage.newBuilder()
                        .setOpen(
                            SessionOpen.newBuilder()
                                .setCapabilityId(commandId)
                                .putAllParameters(parameters)
                                .build()
                        )
                        .build()
                }

                try {
                    requestChannel.send(initialMessage)
                } catch (e: Exception) {
                    Timber.e(e, "Failed to send session ${if (resuming) "resume" else "open"} message")
                    requestChannel.close()
                    connectionManager.disconnect(connectionResult)
                    trySend(ShellSessionEvent.Error(e.message ?: "Failed to start shell session"))
                    break
                }

                pendingReconnect = false

                if (sessionIdDeferred.isCompleted) {
                    scheduleHeartbeat()
                }

                val responseFlow = stub.openSession(requestChannel.receiveAsFlow())

                try {
                    responseFlow.collect { message ->
                        val sessionIdValue = message.sessionId.takeIf { it.isNotBlank() }
                        if (sessionIdValue != null) {
                            resumeState.sessionId = sessionIdValue
                            if (!sessionIdDeferred.isCompleted) {
                                sessionIdDeferred.complete(sessionIdValue)
                                scheduleHeartbeat()
                            }
                        }

                        if (!certificateVerified.get()) {
                            try {
                                verifyServerCertificate(server, connectionResult)
                                certificateVerified.set(true)
                                enrolledServerRepository.updateConnectionMode(serverId, connectionResult.mode)
                            } catch (e: SecurityException) {
                                Timber.e(e, "Certificate verification failed for shell session")
                                trySend(
                                    ShellSessionEvent.Error(
                                        e.message ?: "Server certificate verification failed"
                                    )
                                )
                                cancel("Certificate verification failed", e)
                                return@collect
                            }
                        }

                        when {
                            message.hasReady() -> {
                                val ready = message.ready
                                resumeState.resumeToken = ready.resumeToken
                                trySend(
                                    ShellSessionEvent.Ready(
                                        capabilityId = ready.capabilityId,
                                        message = ready.message
                                    )
                                )
                                resumeAttempts = 0

                                pendingResize?.let { size ->
                                    pendingResize = null
                                    launch {
                                        try {
                                            resizeHandler.get().invoke(size)
                                        } catch (e: Exception) {
                                            Timber.w(e, "Failed to send initial terminal resize")
                                        }
                                    }
                                }
                            }
                            message.hasResumeAck() -> {
                                val ack = message.resumeAck
                                resumeState.resumeToken = ack.resumeToken
                                resumeAttempts = 0
                                trySend(ShellSessionEvent.ResumeAcknowledged(ack.message))
                                pendingResize?.let { size ->
                                    pendingResize = null
                                    launch {
                                        try {
                                            resizeHandler.get().invoke(size)
                                        } catch (e: Exception) {
                                            Timber.w(e, "Failed to send terminal resize after resume")
                                        }
                                    }
                                }
                            }
                            message.hasOutput() -> {
                                val output = message.output
                                val rawBinary = output.binary ?: false
                                val dataBytes = output.data.toByteArray()
                                val decoded = runCatching {
                                    dataBytes.toString(Charsets.UTF_8)
                                }.getOrNull()
                                val hasReplacement = decoded?.contains('\uFFFD') == true

                                val sequence = output.sequence
                                if (output.stderr == true) {
                                    resumeState.lastStderrSequence = max(resumeState.lastStderrSequence, sequence)
                                } else {
                                    resumeState.lastStdoutSequence = max(resumeState.lastStdoutSequence, sequence)
                                }

                                trySend(
                                    ShellSessionEvent.Output(
                                        data = dataBytes,
                                        isError = output.stderr ?: false,
                                        isBinary = rawBinary || hasReplacement,
                                        timestampMs = output.timestampMs,
                                        text = decoded
                                    )
                                )
                            }
                            message.hasExit() -> {
                                val exit = message.exit
                                resumeState.clear()
                                trySend(
                                    ShellSessionEvent.Exit(
                                        exitCode = exit.exitCode,
                                        timedOut = exit.timedOut ?: false,
                                        message = exit.message
                                    )
                                )
                            }
                            message.hasError() -> {
                                val error = message.error
                                trySend(
                                    ShellSessionEvent.Error(
                                        message = error.message,
                                        code = error.code
                                    )
                                )
                            }
                            message.hasHeartbeat() -> {
                                val ack = message.heartbeat
                                trySend(
                                    ShellSessionEvent.Heartbeat(
                                        timestampMs = ack.timestampMs,
                                        latencyHintMs = ack.latencyHintMs
                                    )
                                )
                            }
                            message.hasClosed() -> {
                                val closed = message.closed
                                resumeState.clear()
                                trySend(ShellSessionEvent.Closed(closed.reason))
                            }
                            else -> {
                                Timber.w("Unhandled session message for $commandId: $message")
                            }
                        }
                    }
                } catch (e: StatusException) {
                    failure = e
                    resumePlanned = shouldAttemptResume(e)
                    if (resumePlanned) {
                        resumeAttempts += 1
                        trySend(ShellSessionEvent.Resuming(resumeAttempts))
                    } else if (!userClosed.get()) {
                        Timber.e(e, "Shell session stream failed")
                        trySend(ShellSessionEvent.Error(mapGrpcError(e.status)))
                    }
                } catch (e: StatusRuntimeException) {
                    failure = e
                    resumePlanned = shouldAttemptResume(e)
                    if (resumePlanned) {
                        resumeAttempts += 1
                        trySend(ShellSessionEvent.Resuming(resumeAttempts))
                    } else if (!userClosed.get()) {
                        Timber.e(e, "Shell session stream failed")
                        trySend(ShellSessionEvent.Error(mapGrpcError(e.status)))
                    }
                } catch (e: Exception) {
                    if (e is CancellationException && userClosed.get()) {
                        failure = null
                        resumePlanned = false
                    } else {
                        failure = e
                        resumePlanned = shouldAttemptResume(e)
                        if (resumePlanned) {
                            resumeAttempts += 1
                            trySend(ShellSessionEvent.Resuming(resumeAttempts))
                        } else if (!userClosed.get()) {
                            Timber.e(e, "Shell session terminated unexpectedly")
                            trySend(ShellSessionEvent.Error(e.message ?: "Shell session failed"))
                        }
                    }
                } finally {
                    heartbeatJob?.cancel()
                    requestChannel.close()
                    try {
                        connectionManager.disconnect(connectionResult)
                    } catch (e: Exception) {
                        Timber.w(e, "Failed to disconnect shell session channel")
                    }

                    if (!sessionIdDeferred.isCompleted && failure == null) {
                        sessionIdDeferred.completeExceptionally(
                            IllegalStateException("Session ended before it was ready")
                        )
                    }
                }

                if (!resumePlanned || userClosed.get()) {
                    break
                }

                pendingReconnect = true
            }

            awaitClose {
                userClosed.set(true)
            }
        }

        val shellSession = ShellSession(
            events = events,
            writeInput = { data -> writeHandler.get()(data) },
            resize = { size -> resizeHandler.get()(size) },
            close = { reason -> closeHandler.get()(reason) }
        )

        Result.success(shellSession)
    }

    private fun mapCapabilityKind(protoKind: ProtoCapabilityKind): CommandKind {
        return when (protoKind) {
            ProtoCapabilityKind.CAPABILITY_KIND_SHELL_SCRIPT -> CommandKind.SHELL_SCRIPT
            ProtoCapabilityKind.CAPABILITY_KIND_SHELL_INTERACTIVE -> CommandKind.SHELL_INTERACTIVE
            ProtoCapabilityKind.CAPABILITY_KIND_FILE_TRANSFER -> CommandKind.FILE_TRANSFER
            ProtoCapabilityKind.CAPABILITY_KIND_UNSPECIFIED,
            ProtoCapabilityKind.UNRECOGNIZED -> CommandKind.UNKNOWN
        }
    }

    private fun mapSessionMode(protoMode: ProtoSessionMode): CommandSessionMode {
        return when (protoMode) {
            ProtoSessionMode.SESSION_MODE_ONE_SHOT -> CommandSessionMode.ONE_SHOT
            ProtoSessionMode.SESSION_MODE_REALTIME -> CommandSessionMode.REALTIME
            ProtoSessionMode.SESSION_MODE_UPLOAD -> CommandSessionMode.UPLOAD
            ProtoSessionMode.SESSION_MODE_DOWNLOAD -> CommandSessionMode.DOWNLOAD
            ProtoSessionMode.SESSION_MODE_UNSPECIFIED,
            ProtoSessionMode.UNRECOGNIZED -> CommandSessionMode.UNSPECIFIED
        }
    }

    private fun mapParameterType(protoType: ProtoCapabilityParameterType): ParameterType {
        return when (protoType) {
            ProtoCapabilityParameterType.CAPABILITY_PARAMETER_TYPE_SLIDER -> ParameterType.SLIDER
            ProtoCapabilityParameterType.CAPABILITY_PARAMETER_TYPE_TEXT -> ParameterType.TEXT
            ProtoCapabilityParameterType.CAPABILITY_PARAMETER_TYPE_TOGGLE -> ParameterType.TOGGLE
            ProtoCapabilityParameterType.CAPABILITY_PARAMETER_TYPE_DROPDOWN -> ParameterType.DROPDOWN
            ProtoCapabilityParameterType.CAPABILITY_PARAMETER_TYPE_UNSPECIFIED,
            ProtoCapabilityParameterType.UNRECOGNIZED -> ParameterType.UNSPECIFIED
        }
    }

    private suspend fun verifyServerCertificate(
        server: EnrolledServerEntity,
        connectionResult: ConnectionResult
    ) {
        val provider = connectionResult.serverCertificateProvider ?: return
        val certificate = provider.invoke() ?: return
        val fingerprint = VerificationCodeGenerator.computeFingerprint(certificate.encoded)
        val normalizedComputed = fingerprint
            .removePrefix("SHA256:")
            .replace(":", "")
            .lowercase()
        val normalizedStored = server.certFingerprint
            .removePrefix("SHA256:")
            .replace(":", "")
            .lowercase()

        if (server.certFingerprint.isBlank()) {
            Timber.i("Persisting server fingerprint for ${server.serverName}")
            enrolledServerRepository.updateServerFingerprint(server.serverId, fingerprint)
        } else if (normalizedStored != normalizedComputed) {
            Timber.e(
                "Server certificate fingerprint mismatch: expected=%s actual=%s",
                server.certFingerprint,
                fingerprint
            )
            throw SecurityException("Server certificate fingerprint mismatch")
        }
    }

    private fun mapGrpcError(status: Status): String {
        return when (status.code) {
            Status.Code.UNAUTHENTICATED -> "Authentication failed"
            Status.Code.PERMISSION_DENIED -> "Permission denied"
            Status.Code.NOT_FOUND -> "Capability not found"
            Status.Code.INVALID_ARGUMENT -> "Invalid capability parameters"
            Status.Code.DEADLINE_EXCEEDED -> "Request timeout"
            Status.Code.UNAVAILABLE -> "Server unavailable"
            else -> "Network error: ${status.description ?: status.code}"
        }
    }

    private data class CapabilityResumeState(
        var sessionId: String? = null,
        var resumeToken: String? = null,
        var lastStdoutSequence: Long = 0,
        var lastStderrSequence: Long = 0
    ) {
        fun canResume(): Boolean = !sessionId.isNullOrBlank() && !resumeToken.isNullOrBlank()
        fun clear() {
            resumeToken = null
            lastStdoutSequence = 0
            lastStderrSequence = 0
        }
    }
}

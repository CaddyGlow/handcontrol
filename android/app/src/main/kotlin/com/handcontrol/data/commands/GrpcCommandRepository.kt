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
import com.handcontrol.grpc.RemoteControlGrpcKt
import com.handcontrol.grpc.ServerInfoRequest
import com.handcontrol.grpc.SessionClientMessage
import com.handcontrol.grpc.SessionClose
import com.handcontrol.grpc.SessionHeartbeat
import com.handcontrol.grpc.SessionInput
import com.handcontrol.grpc.SessionMode as ProtoSessionMode
import com.handcontrol.grpc.SessionOpen
import com.handcontrol.grpc.SessionResize
import io.grpc.Status
import io.grpc.StatusException
import java.util.concurrent.atomic.AtomicBoolean
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.channels.Channel
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

    override suspend fun openShellSession(
        serverId: String,
        commandId: String,
        parameters: Map<String, String>,
        terminalSize: TerminalSize?
    ): Result<ShellSession> = withContext(Dispatchers.IO) {
        val server = enrolledServerRepository.getServerById(serverId)
            ?: return@withContext Result.failure(Exception("Server not found: $serverId"))

        Timber.i("Opening realtime shell session to capability $commandId on ${server.serverName}")

        val connectionResult = try {
            connectionManager.connect(server)
        } catch (e: Exception) {
            Timber.e(e, "Failed to connect for shell session")
            return@withContext Result.failure(e)
        }

        val channel = connectionResult.channel
        val stub = RemoteControlGrpcKt.RemoteControlCoroutineStub(channel)

        val requestChannel = Channel<SessionClientMessage>(capacity = Channel.BUFFERED)
        val sessionIdDeferred = CompletableDeferred<String>()
        val certificateVerified = AtomicBoolean(false)
        var heartbeatJob: Job? = null
        val responseFlow = stub.openSession(requestChannel.receiveAsFlow())

        val heartbeatIntervalMs = 15_000L

        val events = callbackFlow<ShellSessionEvent> {
            var pendingResize = terminalSize != null

            val collectorJob = launch {
                try {
                    responseFlow.collect { message ->
                        val sessionIdValue = message.sessionId.takeIf { it.isNotBlank() }
                        if (sessionIdValue != null && !sessionIdDeferred.isCompleted) {
                            sessionIdDeferred.complete(sessionIdValue)
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
                                val sessionId = if (sessionIdDeferred.isCompleted) {
                                    sessionIdDeferred.getCompleted()
                                } else {
                                    null
                                }

                                if (heartbeatJob == null && sessionId != null) {
                                    heartbeatJob = launch {
                                        while (isActive) {
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
                                        }
                                    }
                                }

                                if (pendingResize && sessionId != null && terminalSize != null) {
                                    pendingResize = false
                                    val resizeMessage = SessionClientMessage.newBuilder()
                                        .setSessionId(sessionId)
                                        .setResize(
                                            SessionResize.newBuilder()
                                                .setCols(terminalSize.cols)
                                                .setRows(terminalSize.rows)
                                                .build()
                                        )
                                        .build()
                                    requestChannel.send(resizeMessage)
                                }

                                trySend(
                                    ShellSessionEvent.Ready(
                                        capabilityId = ready.capabilityId,
                                        message = ready.message
                                    )
                                )
                            }
                            message.hasOutput() -> {
                                val output = message.output
                                val rawBinary = output.binary ?: false
                                val dataBytes = output.data.toByteArray()
                                val decoded = try {
                                    dataBytes.toString(Charsets.UTF_8)
                                } catch (_: Exception) {
                                    ""
                                }
                                val hasReplacement = decoded.contains('\uFFFD')
                                val displayText = if (rawBinary && (decoded.isEmpty() || hasReplacement)) {
                                    "[binary output: ${dataBytes.size} bytes]"
                                } else {
                                    decoded
                                }

                                trySend(
                                    ShellSessionEvent.Output(
                                        text = displayText,
                                        isError = output.stderr ?: false,
                                        isBinary = rawBinary && (decoded.isEmpty() || hasReplacement),
                                        timestampMs = output.timestampMs
                                    )
                                )
                            }
                            message.hasExit() -> {
                                val exit = message.exit
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
                                trySend(ShellSessionEvent.Closed(closed.reason))
                            }
                            else -> {
                                Timber.w("Unhandled session message for $commandId: $message")
                            }
                        }
                    }
                } catch (e: StatusException) {
                    Timber.e(e, "Shell session stream failed")
                    trySend(ShellSessionEvent.Error(mapGrpcError(e.status)))
                } catch (e: Exception) {
                    Timber.e(e, "Shell session terminated unexpectedly")
                    trySend(ShellSessionEvent.Error(e.message ?: "Shell session failed"))
                } finally {
                    if (!sessionIdDeferred.isCompleted) {
                        sessionIdDeferred.completeExceptionally(
                            IllegalStateException("Session ended before it was ready")
                        )
                    }
                    close()
                    try {
                        connectionManager.disconnect(connectionResult)
                    } catch (e: Exception) {
                        Timber.w(e, "Failed to disconnect shell session channel")
                    }
                }
            }

            awaitClose {
                heartbeatJob?.cancel()
                collectorJob.cancel()
                requestChannel.close()
            }
        }

        val openMessage = SessionClientMessage.newBuilder()
            .setOpen(
                SessionOpen.newBuilder()
                    .setCapabilityId(commandId)
                    .putAllParameters(parameters)
                    .build()
            )
            .build()

        try {
            requestChannel.send(openMessage)
        } catch (e: Exception) {
            Timber.e(e, "Failed to send session open message")
            requestChannel.close()
            connectionManager.disconnect(connectionResult)
            return@withContext Result.failure(e)
        }

        val writeInput: suspend (ByteArray) -> Unit = { data ->
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

        val resize: suspend (TerminalSize) -> Unit = { size ->
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

        val close: suspend (String?) -> Unit = { reason ->
            val sessionId = sessionIdDeferred.await()
            val closeMessage = SessionClientMessage.newBuilder()
                .setSessionId(sessionId)
                .setClose(
                    SessionClose.newBuilder()
                        .apply { reason?.let { setReason(it) } }
                        .build()
                )
                .build()
            requestChannel.send(closeMessage)
            requestChannel.close()
        }

        Result.success(
            ShellSession(
                events = events,
                writeInput = writeInput,
                resize = resize,
                close = close
            )
        )
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
}

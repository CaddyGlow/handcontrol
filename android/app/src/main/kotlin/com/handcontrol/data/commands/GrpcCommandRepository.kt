package com.handcontrol.data.commands

import com.handcontrol.core.network.ConnectionResult
import com.handcontrol.core.network.RelayConnectionException
import com.handcontrol.core.network.ServerConnectionManager
import com.handcontrol.data.database.EnrolledServerRepository
import com.handcontrol.data.database.EnrolledServerEntity
import com.handcontrol.core.security.VerificationCodeGenerator
import com.handcontrol.grpc.ExecuteCommandRequest
import com.handcontrol.grpc.ListCommandsRequest
import com.handcontrol.grpc.ParameterType as ProtoParameterType
import com.handcontrol.grpc.RemoteControlGrpcKt
import com.handcontrol.grpc.ServerInfoRequest
import io.grpc.Status
import io.grpc.StatusException
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.catch
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.onCompletion
import timber.log.Timber
import javax.inject.Inject
import javax.inject.Singleton

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

            Timber.i("Connecting to server ${server.serverName} to list commands")
            val connectionResult = connectionManager.connect(server)
            val result = try {
                val channel = connectionResult.channel

                Timber.d("Connected via ${connectionResult.mode} mode")
                val stub = RemoteControlGrpcKt.RemoteControlCoroutineStub(channel)

                val request = ListCommandsRequest.newBuilder().build()
                val response = stub.listCommands(request)

                verifyServerCertificate(server, connectionResult)

                // Persist successful connection mode
                enrolledServerRepository.updateConnectionMode(serverId, connectionResult.mode)

                val commands = response.commandsList.map { protoCommand ->
                    Command(
                        id = protoCommand.id,
                        name = protoCommand.name,
                        description = protoCommand.description,
                        icon = protoCommand.icon,
                        tags = protoCommand.tagsList,
                        parameters = protoCommand.parametersList.map { protoParam ->
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
                        },
                        requiresConfirmation = if (protoCommand.hasRequiresConfirmation()) protoCommand.requiresConfirmation else false,
                        showOutput = if (protoCommand.hasShowOutput()) protoCommand.showOutput else true
                    )
                }
                Timber.i("Loaded ${commands.size} commands from server")
                Result.success(commands)
            } finally {
                connectionManager.disconnect(connectionResult)
            }

            result
        } catch (e: RelayConnectionException) {
            Timber.e(e, "Relay connection failed: ${e.shortMessage}")
            Result.failure(Exception(e.userMessage, e))
        } catch (e: StatusException) {
            Timber.e(e, "Failed to list commands")
            Result.failure(Exception(mapGrpcError(e.status)))
        } catch (e: SecurityException) {
            Timber.e(e, "Server certificate verification failed")
            Result.failure(e)
        } catch (e: Exception) {
            Timber.e(e, "Failed to list commands")
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

        Timber.i("Connecting to server ${server.serverName} to execute command: $commandId")
        val connectionResult = connectionManager.connect(server)
        val channel = connectionResult.channel

        Timber.d("Connected via ${connectionResult.mode} mode")
        val stub = RemoteControlGrpcKt.RemoteControlCoroutineStub(channel)

        val request = ExecuteCommandRequest.newBuilder()
            .setCommandId(commandId)
            .putAllParameters(parameters)
            .build()

        Timber.i("Executing command: $commandId with parameters: $parameters")

        return stub.executeCommand(request)
            .map { response ->
                when {
                    response.hasStdout() -> {
                        Timber.d("Command stdout: ${response.stdout}")
                        CommandExecutionResult.Output(response.stdout, isError = false)
                    }
                    response.hasStderr() -> {
                        Timber.w("Command stderr: ${response.stderr}")
                        CommandExecutionResult.Output(response.stderr, isError = true)
                    }
                    response.hasExitCode() -> {
                        Timber.i("Command finished with exit code: ${response.exitCode}")
                        CommandExecutionResult.ExitCode(response.exitCode)
                    }
                    response.hasError() -> {
                        Timber.e("Command error: ${response.error}")
                        CommandExecutionResult.Error(response.error)
                    }
                    else -> {
                        Timber.w("Unknown response type")
                        CommandExecutionResult.Error("Unknown response type")
                    }
                }
            }
            .catch { e ->
                Timber.e(e, "Command execution failed")
                when (e) {
                    is StatusException -> emit(CommandExecutionResult.Error(mapGrpcError(e.status)))
                    is SecurityException -> emit(CommandExecutionResult.Error(e.message ?: "Security verification failed"))
                    else -> emit(CommandExecutionResult.Error(e.message ?: "Unknown error"))
                }
            }
            .onCompletion { cause ->
                // Disconnect channel when flow completes (success or error)
                Timber.d("Command execution flow completed (cause: $cause)")

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

    private fun mapParameterType(protoType: ProtoParameterType): ParameterType {
        return when (protoType) {
            ProtoParameterType.PARAMETER_TYPE_SLIDER -> ParameterType.SLIDER
            ProtoParameterType.PARAMETER_TYPE_TEXT -> ParameterType.TEXT
            ProtoParameterType.PARAMETER_TYPE_TOGGLE -> ParameterType.TOGGLE
            ProtoParameterType.PARAMETER_TYPE_DROPDOWN -> ParameterType.DROPDOWN
            else -> ParameterType.UNSPECIFIED
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
            Status.Code.NOT_FOUND -> "Command not found"
            Status.Code.INVALID_ARGUMENT -> "Invalid command parameters"
            Status.Code.DEADLINE_EXCEEDED -> "Request timeout"
            Status.Code.UNAVAILABLE -> "Server unavailable"
            else -> "Network error: ${status.description ?: status.code}"
        }
    }
}

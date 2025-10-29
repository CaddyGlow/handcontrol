package com.handcontrol.data.commands

import com.handcontrol.core.network.GrpcChannelFactory
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
import timber.log.Timber
import javax.inject.Inject
import javax.inject.Singleton

@Singleton
class GrpcCommandRepository @Inject constructor(
    private val channelFactory: GrpcChannelFactory
) : CommandRepository {

    override suspend fun getServerInfo(host: String, port: Int): Result<ServerInfo> {
        return try {
            val channel = channelFactory.createChannel(host, port)
            val stub = RemoteControlGrpcKt.RemoteControlCoroutineStub(channel)

            val request = ServerInfoRequest.newBuilder().build()
            val response = stub.getServerInfo(request)

            channelFactory.shutdownChannel(channel)

            Result.success(
                ServerInfo(
                    serverId = response.serverId,
                    hostname = response.hostname,
                    version = response.version,
                    os = response.os
                )
            )
        } catch (e: StatusException) {
            Timber.e(e, "Failed to get server info")
            Result.failure(Exception(mapGrpcError(e.status)))
        } catch (e: Exception) {
            Timber.e(e, "Failed to get server info")
            Result.failure(e)
        }
    }

    override suspend fun listCommands(host: String, port: Int): Result<List<Command>> {
        return try {
            val channel = channelFactory.createChannel(host, port)
            val stub = RemoteControlGrpcKt.RemoteControlCoroutineStub(channel)

            val request = ListCommandsRequest.newBuilder().build()
            val response = stub.listCommands(request)

            channelFactory.shutdownChannel(channel)

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
                            labelOff = if (protoParam.hasLabelOff()) protoParam.labelOff else null
                        )
                    }
                )
            }

            Timber.i("Loaded ${commands.size} commands from server")
            Result.success(commands)
        } catch (e: StatusException) {
            Timber.e(e, "Failed to list commands")
            Result.failure(Exception(mapGrpcError(e.status)))
        } catch (e: Exception) {
            Timber.e(e, "Failed to list commands")
            Result.failure(e)
        }
    }

    override suspend fun executeCommand(
        host: String,
        port: Int,
        commandId: String,
        parameters: Map<String, String>
    ): Flow<CommandExecutionResult> {
        val channel = channelFactory.createChannel(host, port)
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
                        channelFactory.shutdownChannel(channel)
                        CommandExecutionResult.ExitCode(response.exitCode)
                    }
                    response.hasError() -> {
                        Timber.e("Command error: ${response.error}")
                        channelFactory.shutdownChannel(channel)
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
                channelFactory.shutdownChannel(channel)
                when (e) {
                    is StatusException -> emit(CommandExecutionResult.Error(mapGrpcError(e.status)))
                    else -> emit(CommandExecutionResult.Error(e.message ?: "Unknown error"))
                }
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

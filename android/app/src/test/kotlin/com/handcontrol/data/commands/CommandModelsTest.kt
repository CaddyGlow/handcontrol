package com.handcontrol.data.commands

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class CommandModelsTest {

    @Test
    fun `Command model holds all required fields`() {
        val param = CommandParameter(
            name = "volume",
            type = ParameterType.SLIDER,
            description = "Volume level",
            min = 0,
            max = 100,
            defaultValue = "50"
        )

        val command = Command(
            id = "test-cmd",
            name = "Test Command",
            description = "A test command",
            icon = "test_icon",
            tags = listOf("test", "sample"),
            parameters = listOf(param),
            requiresConfirmation = true,
            showOutput = false,
            privileged = true,
            kind = CommandKind.SHELL_SCRIPT,
            sessionMode = CommandSessionMode.ONE_SHOT
        )

        assertEquals("test-cmd", command.id)
        assertEquals("Test Command", command.name)
        assertEquals("A test command", command.description)
        assertEquals("test_icon", command.icon)
        assertEquals(2, command.tags.size)
        assertEquals(1, command.parameters.size)
        assertEquals(param, command.parameters[0])
        assertTrue(command.requiresConfirmation)
        assertFalse(command.showOutput)
        assertTrue(command.privileged)
        assertEquals(CommandKind.SHELL_SCRIPT, command.kind)
        assertEquals(CommandSessionMode.ONE_SHOT, command.sessionMode)
    }

    @Test
    fun `CommandParameter slider type includes min max`() {
        val param = CommandParameter(
            name = "brightness",
            type = ParameterType.SLIDER,
            description = "Screen brightness",
            min = 0,
            max = 255,
            defaultValue = "128"
        )

        assertEquals(ParameterType.SLIDER, param.type)
        assertEquals(0, param.min)
        assertEquals(255, param.max)
        assertEquals("128", param.defaultValue)
    }

    @Test
    fun `CommandParameter toggle type includes labels`() {
        val param = CommandParameter(
            name = "enabled",
            type = ParameterType.TOGGLE,
            description = "Enable feature",
            labelOn = "Enabled",
            labelOff = "Disabled"
        )

        assertEquals(ParameterType.TOGGLE, param.type)
        assertEquals("Enabled", param.labelOn)
        assertEquals("Disabled", param.labelOff)
    }

    @Test
    fun `CommandParameter dropdown includes options`() {
        val param = CommandParameter(
            name = "mode",
            type = ParameterType.DROPDOWN,
            description = "Operation mode",
            options = listOf("Fast", "Normal", "Slow")
        )

        assertEquals(ParameterType.DROPDOWN, param.type)
        assertEquals(3, param.options.size)
        assertTrue(param.options.contains("Fast"))
        assertTrue(param.options.contains("Normal"))
        assertTrue(param.options.contains("Slow"))
    }

    @Test
    fun `CommandExecutionResult Output can be stdout or stderr`() {
        val stdout = CommandExecutionResult.Output("Hello", isError = false)
        val stderr = CommandExecutionResult.Output("Error!", isError = true)

        assertFalse(stdout.isError)
        assertTrue(stderr.isError)
        assertEquals("Hello", stdout.text)
        assertEquals("Error!", stderr.text)
    }

    @Test
    fun `CommandExecutionResult ExitCode holds code`() {
        val success = CommandExecutionResult.ExitCode(0)
        val failure = CommandExecutionResult.ExitCode(1)

        assertEquals(0, success.code)
        assertEquals(1, failure.code)
    }

    @Test
    fun `CommandExecutionResult Error holds message`() {
        val error = CommandExecutionResult.Error("Connection failed")

        assertEquals("Connection failed", error.message)
    }

    @Test
    fun `ServerInfo holds all metadata`() {
        val info = ServerInfo(
            serverId = "server-123",
            hostname = "my-laptop",
            version = "0.1.0",
            os = "Linux"
        )

        assertEquals("server-123", info.serverId)
        assertEquals("my-laptop", info.hostname)
        assertEquals("0.1.0", info.version)
        assertEquals("Linux", info.os)
    }

    @Test
    fun `ParameterType enum covers all types`() {
        val types = ParameterType.values()

        assertTrue(types.contains(ParameterType.SLIDER))
        assertTrue(types.contains(ParameterType.TEXT))
        assertTrue(types.contains(ParameterType.TOGGLE))
        assertTrue(types.contains(ParameterType.DROPDOWN))
        assertTrue(types.contains(ParameterType.UNSPECIFIED))
        assertEquals(5, types.size)
    }
}

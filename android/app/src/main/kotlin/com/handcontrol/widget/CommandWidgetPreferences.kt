package com.handcontrol.widget

import android.content.Context

/**
 * Helper class to manage widget configuration preferences.
 * Stores mapping of widgetId -> (serverId, commandId, commandName, serverName)
 */
object CommandWidgetPreferences {

    private const val PREFS_NAME = "com.handcontrol.widget.prefs"
    private const val KEY_SERVER_ID = "widget_%d_server_id"
    private const val KEY_COMMAND_ID = "widget_%d_command_id"
    private const val KEY_COMMAND_NAME = "widget_%d_command_name"
    private const val KEY_SERVER_NAME = "widget_%d_server_name"
    private const val KEY_SHOW_SERVER_NAME = "widget_%d_show_server_name"

    data class WidgetConfig(
        val serverId: String,
        val commandId: String,
        val commandName: String,
        val serverName: String,
        val showServerName: Boolean = false
    )

    fun saveWidgetConfig(context: Context, widgetId: Int, config: WidgetConfig) {
        val prefs = context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)
        prefs.edit().apply {
            putString(KEY_SERVER_ID.format(widgetId), config.serverId)
            putString(KEY_COMMAND_ID.format(widgetId), config.commandId)
            putString(KEY_COMMAND_NAME.format(widgetId), config.commandName)
            putString(KEY_SERVER_NAME.format(widgetId), config.serverName)
            putBoolean(KEY_SHOW_SERVER_NAME.format(widgetId), config.showServerName)
            apply()
        }
    }

    fun getWidgetConfig(context: Context, widgetId: Int): WidgetConfig? {
        val prefs = context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)
        val serverId = prefs.getString(KEY_SERVER_ID.format(widgetId), null) ?: return null
        val commandId = prefs.getString(KEY_COMMAND_ID.format(widgetId), null) ?: return null
        val commandName = prefs.getString(KEY_COMMAND_NAME.format(widgetId), null) ?: return null
        val serverName = prefs.getString(KEY_SERVER_NAME.format(widgetId), null) ?: return null
        val showServerName = prefs.getBoolean(KEY_SHOW_SERVER_NAME.format(widgetId), false)

        return WidgetConfig(
            serverId = serverId,
            commandId = commandId,
            commandName = commandName,
            serverName = serverName,
            showServerName = showServerName
        )
    }

    fun deleteWidgetConfig(context: Context, widgetId: Int) {
        val prefs = context.getSharedPreferences(PREFS_NAME, Context.MODE_PRIVATE)
        prefs.edit().apply {
            remove(KEY_SERVER_ID.format(widgetId))
            remove(KEY_COMMAND_ID.format(widgetId))
            remove(KEY_COMMAND_NAME.format(widgetId))
            remove(KEY_SERVER_NAME.format(widgetId))
            remove(KEY_SHOW_SERVER_NAME.format(widgetId))
            apply()
        }
    }
}

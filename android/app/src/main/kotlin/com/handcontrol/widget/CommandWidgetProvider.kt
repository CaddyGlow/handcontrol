package com.handcontrol.widget

import android.app.PendingIntent
import android.appwidget.AppWidgetManager
import android.appwidget.AppWidgetProvider
import android.content.Context
import android.content.Intent
import android.widget.RemoteViews
import com.handcontrol.R

/**
 * Widget provider for command buttons on the home screen.
 * Allows users to execute commands with a single tap from their home screen.
 */
class CommandWidgetProvider : AppWidgetProvider() {

    override fun onUpdate(
        context: Context,
        appWidgetManager: AppWidgetManager,
        appWidgetIds: IntArray
    ) {
        // Update all widgets
        for (appWidgetId in appWidgetIds) {
            updateAppWidget(context, appWidgetManager, appWidgetId)
        }
    }

    override fun onDeleted(context: Context, appWidgetIds: IntArray) {
        // Clean up widget preferences when widgets are deleted
        for (appWidgetId in appWidgetIds) {
            CommandWidgetPreferences.deleteWidgetConfig(context, appWidgetId)
        }
    }

    override fun onEnabled(context: Context) {
        // First widget added - perform any one-time setup if needed
    }

    override fun onDisabled(context: Context) {
        // Last widget removed - perform any cleanup if needed
    }

    override fun onReceive(context: Context, intent: Intent) {
        super.onReceive(context, intent)

        when (intent.action) {
            ACTION_WIDGET_CLICK -> {
                val appWidgetId = intent.getIntExtra(
                    AppWidgetManager.EXTRA_APPWIDGET_ID,
                    AppWidgetManager.INVALID_APPWIDGET_ID
                )
                if (appWidgetId != AppWidgetManager.INVALID_APPWIDGET_ID) {
                    handleWidgetClick(context, appWidgetId)
                }
            }
        }
    }

    private fun handleWidgetClick(context: Context, appWidgetId: Int) {
        val config = CommandWidgetPreferences.getWidgetConfig(context, appWidgetId)
        if (config == null) {
            // Widget not configured - open config activity
            val configIntent = Intent(context, CommandWidgetConfigActivity::class.java).apply {
                putExtra(AppWidgetManager.EXTRA_APPWIDGET_ID, appWidgetId)
                flags = Intent.FLAG_ACTIVITY_NEW_TASK
            }
            context.startActivity(configIntent)
            return
        }

        // TODO: Implement command execution logic
        // For now, open the app to the command
        val appIntent = context.packageManager.getLaunchIntentForPackage(context.packageName)?.apply {
            flags = Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP
            // Add extras for deep linking to the command
            putExtra("server_id", config.serverId)
            putExtra("command_id", config.commandId)
        }
        appIntent?.let { context.startActivity(it) }
    }

    companion object {
        private const val ACTION_WIDGET_CLICK = "com.handcontrol.widget.WIDGET_CLICK"

        fun updateAppWidget(
            context: Context,
            appWidgetManager: AppWidgetManager,
            appWidgetId: Int
        ) {
            val config = CommandWidgetPreferences.getWidgetConfig(context, appWidgetId)

            val views = RemoteViews(context.packageName, R.layout.widget_command_button)

            if (config != null) {
                // Set command name
                views.setTextViewText(R.id.widget_command_name, config.commandName)

                // Optionally show server name
                if (config.showServerName) {
                    views.setTextViewText(R.id.widget_server_name, config.serverName)
                    views.setViewVisibility(R.id.widget_server_name, android.view.View.VISIBLE)
                } else {
                    views.setViewVisibility(R.id.widget_server_name, android.view.View.GONE)
                }

                // Hide overlays
                views.setViewVisibility(R.id.widget_loading, android.view.View.GONE)
                views.setViewVisibility(R.id.widget_success, android.view.View.GONE)
                views.setViewVisibility(R.id.widget_error, android.view.View.GONE)

                // Set up click intent
                val clickIntent = Intent(context, CommandWidgetProvider::class.java).apply {
                    action = ACTION_WIDGET_CLICK
                    putExtra(AppWidgetManager.EXTRA_APPWIDGET_ID, appWidgetId)
                }
                val pendingIntent = PendingIntent.getBroadcast(
                    context,
                    appWidgetId,
                    clickIntent,
                    PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
                )
                views.setOnClickPendingIntent(R.id.widget_content, pendingIntent)
            } else {
                // Widget not configured - show placeholder
                views.setTextViewText(R.id.widget_command_name, "Tap to configure")
                views.setViewVisibility(R.id.widget_server_name, android.view.View.GONE)

                // Set up config intent
                val configIntent = Intent(context, CommandWidgetConfigActivity::class.java).apply {
                    putExtra(AppWidgetManager.EXTRA_APPWIDGET_ID, appWidgetId)
                }
                val pendingIntent = PendingIntent.getActivity(
                    context,
                    appWidgetId,
                    configIntent,
                    PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
                )
                views.setOnClickPendingIntent(R.id.widget_content, pendingIntent)
            }

            appWidgetManager.updateAppWidget(appWidgetId, views)
        }
    }
}

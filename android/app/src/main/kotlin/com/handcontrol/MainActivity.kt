package com.handcontrol

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dagger.hilt.android.AndroidEntryPoint
import com.handcontrol.data.settings.SettingsRepository
import com.handcontrol.data.settings.Theme
import com.handcontrol.ui.theme.HandControlTheme
import com.handcontrol.navigation.HandControlNavHost
import javax.inject.Inject

@AndroidEntryPoint
class MainActivity : ComponentActivity() {
    @Inject
    lateinit var settingsRepository: SettingsRepository

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContent {
            HandControlApp(settingsRepository)
        }
    }
}

@Composable
fun HandControlApp(settingsRepository: SettingsRepository) {
    val settings by settingsRepository.settings.collectAsStateWithLifecycle(
        initialValue = com.handcontrol.data.settings.AppSettings()
    )

    HandControlTheme(
        darkTheme = when (settings.theme) {
            Theme.LIGHT -> false
            Theme.DARK -> true
            Theme.SYSTEM -> isSystemInDarkTheme()
        }
    ) {
        HandControlNavHost()
    }
}


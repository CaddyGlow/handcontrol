package com.handcontrol.feature.welcome

import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.foundation.layout.padding

object WelcomeRoute {
    const val route: String = "welcome"

    @OptIn(ExperimentalMaterial3Api::class)
    @Composable
    fun Content() {
        Scaffold(
            topBar = {
                TopAppBar(title = { Text(text = "HandControl") })
            }
        ) { innerPadding ->
            WelcomeScreen(modifier = Modifier.padding(innerPadding))
        }
    }
}

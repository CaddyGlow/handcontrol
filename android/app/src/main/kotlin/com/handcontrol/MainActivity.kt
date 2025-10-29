package com.handcontrol

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.runtime.Composable
import androidx.compose.ui.tooling.preview.Preview
import dagger.hilt.android.AndroidEntryPoint
import com.handcontrol.ui.theme.HandControlTheme
import com.handcontrol.navigation.HandControlNavHost

@AndroidEntryPoint
class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContent {
            HandControlApp()
        }
    }
}

@Composable
fun HandControlApp() {
    HandControlTheme {
        HandControlNavHost()
    }
}

@Preview(showBackground = true, showSystemUi = true)
@Composable
private fun HandControlPreview() {
    HandControlApp()
}

package com.handcontrol

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.tooling.preview.Preview
import com.handcontrol.ui.theme.HandControlTheme

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContent {
            HandControlApp()
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun HandControlApp() {
    HandControlTheme {
        Scaffold(
            topBar = {
                TopAppBar(title = { Text(text = "HandControl") })
            }
        ) { innerPadding ->
            WelcomeContent(modifier = Modifier.padding(innerPadding))
        }
    }
}

@Composable
private fun WelcomeContent(modifier: Modifier = Modifier) {
    Text(
        text = "Secure desktop control from your Android device.",
        modifier = modifier
    )
}

@Preview(showBackground = true, showSystemUi = true)
@Composable
private fun HandControlPreview() {
    HandControlApp()
}

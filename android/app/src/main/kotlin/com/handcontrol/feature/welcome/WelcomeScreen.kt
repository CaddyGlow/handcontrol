package com.handcontrol.feature.welcome

import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.tooling.preview.Preview
import com.handcontrol.ui.theme.HandControlTheme

@Composable
fun WelcomeScreen(modifier: Modifier = Modifier) {
    Text(
        text = "Secure desktop control from your Android device.",
        modifier = modifier
    )
}

@Preview(showBackground = true, showSystemUi = true)
@Composable
private fun WelcomeScreenPreview() {
    HandControlTheme {
        WelcomeScreen()
    }
}

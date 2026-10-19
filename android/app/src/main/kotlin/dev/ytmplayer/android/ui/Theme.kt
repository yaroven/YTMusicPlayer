package dev.ytmplayer.android.ui

import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color

/** Dark only, like YouTube Music; the accent is the iPhone app's red. */
private val colors = darkColorScheme(
    primary = Color(0xFFFF3B30),
    onPrimary = Color.White,
    secondary = Color(0xFFFF3B30),
    background = Color.Black,
    surface = Color.Black,
    surfaceContainer = Color(0xFF1C1C1E),
    surfaceContainerHigh = Color(0xFF2A2A2C),
    onSurfaceVariant = Color(0xFFA0A0A5),
)

@Composable
fun YtmTheme(content: @Composable () -> Unit) {
    MaterialTheme(colorScheme = colors, content = content)
}

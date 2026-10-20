package dev.ytmplayer.android.ui

import android.os.Build
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.AccountCircle
import androidx.compose.material.icons.filled.PlayArrow
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.darkColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.blur
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.draw.scale
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import coil.compose.AsyncImage
import dev.ytmplayer.android.App
import dev.ytmplayer.android.R

/**
 * YouTube Music's look: near-black, white text, grey secondary text,
 * translucent chips, white for what's selected, red only in the logo.
 */
object YT {
    val background = Color(0xFF030303)
    val surface = Color(0xFF212121)
    val raised = Color.White.copy(alpha = 0.1f)
    val secondary = Color(0xFFAAAAAA)
    val red = Color(0xFFFF0000)
}

private val colors = darkColorScheme(
    primary = Color.White,
    onPrimary = Color.Black,
    secondary = Color.White,
    background = YT.background,
    surface = YT.background,
    surfaceContainer = YT.surface,
    surfaceContainerHigh = Color(0xFF2A2A2A),
    surfaceContainerHighest = Color(0xFF333333),
    onSurface = Color.White,
    onSurfaceVariant = YT.secondary,
    secondaryContainer = Color.Transparent,
)

@Composable
fun YtmTheme(content: @Composable () -> Unit) {
    MaterialTheme(colorScheme = colors, content = content)
}

/** The "▶ Music" logo. */
@Composable
fun YTLogo() {
    Row(verticalAlignment = Alignment.CenterVertically) {
        Box(Modifier.size(28.dp).clip(CircleShape).background(YT.red), contentAlignment = Alignment.Center) {
            Box(Modifier.size(18.dp).border(1.6.dp, Color.White, CircleShape))
            Icon(Icons.Default.PlayArrow, null, Modifier.size(14.dp).offset(x = 0.5.dp), tint = Color.White)
        }
        Spacer(Modifier.width(4.dp))
        Text("Music", fontSize = 22.sp, fontWeight = FontWeight.Bold, letterSpacing = (-0.6).sp)
    }
}

/** The account's picture (or a person), which opens Settings. */
@Composable
fun AvatarButton() {
    val account = App.instance.account
    Box(
        Modifier.size(32.dp).clip(CircleShape).clickable { Router.open(Route.Settings) },
        contentAlignment = Alignment.Center,
    ) {
        val photo = account.photo
        if (photo != null) {
            AsyncImage(photo, stringResource(R.string.settings), Modifier.fillMaxSize(), contentScale = ContentScale.Crop)
        } else {
            Icon(Icons.Default.AccountCircle, stringResource(R.string.settings), Modifier.fillMaxSize(), tint = YT.secondary)
        }
    }
}

/** A rounded chip: Home's moods, search kinds, library filters. */
@Composable
fun ChipButton(title: String, selected: Boolean, onClick: () -> Unit) {
    Box(
        Modifier
            .clip(RoundedCornerShape(8.dp))
            .background(if (selected) Color.White else YT.raised)
            .clickable(onClick = onClick)
            .padding(horizontal = 14.dp, vertical = 8.dp),
    ) {
        Text(title, fontSize = 15.sp, fontWeight = FontWeight.Medium, color = if (selected) Color.Black else Color.White)
    }
}

/** "Play all" and friends: an outlined pill. */
@Composable
fun OutlinePill(title: String, onClick: () -> Unit) {
    Box(
        Modifier
            .clip(CircleShape)
            .border(1.dp, Color.White.copy(alpha = 0.3f), CircleShape)
            .clickable(onClick = onClick)
            .padding(horizontal = 14.dp, vertical = 7.dp),
    ) {
        Text(title, fontSize = 14.sp, fontWeight = FontWeight.Medium)
    }
}

/** A round icon button with a translucent fill (page and player actions). */
@Composable
fun CircleIcon(icon: ImageVector, label: String, size: Dp = 48.dp, filled: Boolean = false, enabled: Boolean = true, onClick: () -> Unit) {
    Box(
        Modifier
            .size(size)
            .clip(CircleShape)
            .background(if (filled) Color.White else YT.raised)
            .clickable(enabled = enabled, onClick = onClick)
            .alpha(if (enabled) 1f else 0.4f),
        contentAlignment = Alignment.Center,
    ) {
        Icon(icon, label, Modifier.size(size * 0.5f), tint = if (filled) Color.Black else Color.White)
    }
}

/**
 * The colours of a picture, blurred, fading into the background at the top
 * of a screen (YouTube Music's ambient header). Android 12+ blurs; older
 * versions get the gradient only.
 */
@Composable
fun AmbientBackdrop(url: String?, height: Dp = 420.dp) {
    Box(Modifier.fillMaxWidth().height(height).clipToBounds()) {
        if (url != null && Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            AsyncImage(
                url, null, Modifier.fillMaxSize().scale(1.6f).blur(70.dp).alpha(0.55f),
                contentScale = ContentScale.Crop,
            )
        }
        Box(Modifier.fillMaxSize().background(Brush.verticalGradient(0.4f to Color.Transparent, 1f to YT.background)))
    }
}

/** White on a faint track, like YouTube Music's scrubber. */
@Composable
fun ytSlider() = androidx.compose.material3.SliderDefaults.colors(
    thumbColor = Color.White,
    activeTrackColor = Color.White,
    inactiveTrackColor = Color.White.copy(alpha = 0.2f),
    activeTickColor = Color.Transparent,
    inactiveTickColor = Color.Transparent,
)

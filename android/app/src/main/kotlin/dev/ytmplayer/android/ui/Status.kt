package dev.ytmplayer.android.ui

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.expandVertically
import androidx.compose.animation.shrinkVertically
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.CloudOff
import androidx.compose.material.icons.filled.Error
import androidx.compose.material.icons.filled.HourglassBottom
import androidx.compose.material.icons.filled.Lock
import androidx.compose.material.icons.filled.Wifi
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.Stable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import dev.ytmplayer.android.App
import dev.ytmplayer.android.Problem
import dev.ytmplayer.android.R
import dev.ytmplayer.android.describe
import dev.ytmplayer.android.problem
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.delay

/**
 * Something loaded from the network for a screen: the value, the last
 * failure, and whether a request is running. It loads again by itself when
 * the connection comes back after a failure.
 */
@Stable
class Load<T> {
    var value by mutableStateOf<T?>(null)
        internal set
    var error by mutableStateOf<Throwable?>(null)
        internal set
    var loading by mutableStateOf(true)
        internal set
    internal var attempt by mutableIntStateOf(0)

    fun retry() {
        attempt++
    }

    /** More of the same value arrived (a long list's next part). */
    fun update(newer: T) {
        value = newer
        key?.let { recent.put(it, newer as Any) }
    }

    internal var key: Any? = null
}

/**
 * The last screens' data, so going back (or switching tabs) shows them at
 * once instead of loading again; pull to refresh or Retry loads afresh.
 */
internal val recent = android.util.LruCache<Any, Any>(16)

@Suppress("UNCHECKED_CAST")
@Composable
fun <T> rememberLoad(key: Any?, load: suspend () -> T): Load<T> {
    val state = remember(key) {
        Load<T>().apply {
            this.key = key
            (key?.let { recent.get(it) } as T?)?.let {
                value = it
                loading = false
            }
        }
    }
    val online by App.instance.network.online.collectAsState()
    LaunchedEffect(key, state.attempt) {
        if (state.attempt == 0 && state.value != null) return@LaunchedEffect
        state.loading = true
        try {
            state.value = load()
            key?.let { k -> state.value?.let { recent.put(k, it) } }
            state.error = null
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            state.error = e
        } finally {
            state.loading = false
        }
    }
    // Back online after a failure: try again without being asked.
    LaunchedEffect(online) {
        if (online && state.error != null && !state.loading) state.retry()
    }
    return state
}

/**
 * The screen's content once loaded; a spinner (with a "slow connection"
 * hint after a while) before; a clear error with Retry if it failed.
 */
@Composable
fun <T> Loaded(state: Load<T>, modifier: Modifier = Modifier, content: @Composable (T) -> Unit) {
    val value = state.value
    when {
        value != null -> content(value)
        state.error != null && !state.loading -> ErrorState(state.error!!, modifier, onRetry = state::retry)
        else -> Loading(modifier)
    }
}

/** A spinner; after a few seconds it says why it takes so long. */
@Composable
fun Loading(modifier: Modifier = Modifier) {
    val online by App.instance.network.online.collectAsState()
    var slow by remember { mutableStateOf(false) }
    LaunchedEffect(Unit) {
        delay(SLOW_HINT_MS)
        slow = true
    }
    Column(
        modifier.fillMaxSize().padding(32.dp),
        verticalArrangement = Arrangement.Center,
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        CircularProgressIndicator()
        if (!online || slow) {
            Spacer(Modifier.height(16.dp))
            Text(
                stringResource(if (!online) R.string.waiting_for_connection else R.string.slow_loading),
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                textAlign = TextAlign.Center,
            )
        }
    }
}

@Composable
fun ErrorState(error: Throwable, modifier: Modifier = Modifier, onRetry: (() -> Unit)? = null) {
    val context = LocalContext.current
    val online by App.instance.network.online.collectAsState()
    val problem = error.problem(online)
    val (icon, title, detail) = when (problem) {
        Problem.Offline -> Triple(Icons.Default.CloudOff, stringResource(R.string.offline_title), stringResource(R.string.offline_detail))
        Problem.Slow -> Triple(Icons.Default.HourglassBottom, stringResource(R.string.slow_title), stringResource(R.string.slow_detail))
        Problem.Server -> Triple(Icons.Default.Error, stringResource(R.string.server_title), stringResource(R.string.server_detail))
        Problem.SignIn -> Triple(Icons.Default.Lock, stringResource(R.string.sign_in_title), error.describe(context, online))
        Problem.Other -> Triple(Icons.Default.Error, stringResource(R.string.error_title), error.describe(context, online))
    }
    Message(icon, title, detail, modifier) {
        if (onRetry != null) Button(onClick = onRetry) { Text(stringResource(R.string.retry)) }
        if (problem == Problem.Offline && App.instance.library.downloads.isNotEmpty()) {
            OutlinedButton(onClick = { Router.openDownloads() }) { Text(stringResource(R.string.open_downloads)) }
        }
    }
}

/** An icon, a title, a line of detail and optional buttons, centred. */
@Composable
fun Message(
    icon: ImageVector,
    title: String,
    detail: String? = null,
    modifier: Modifier = Modifier,
    actions: @Composable () -> Unit = {},
) {
    Column(
        modifier.fillMaxSize().padding(32.dp),
        verticalArrangement = Arrangement.Center,
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Icon(icon, null, Modifier.size(48.dp), tint = MaterialTheme.colorScheme.onSurfaceVariant)
        Spacer(Modifier.height(12.dp))
        Text(title, style = MaterialTheme.typography.titleMedium, textAlign = TextAlign.Center)
        if (!detail.isNullOrEmpty()) {
            Spacer(Modifier.height(6.dp))
            Text(
                detail,
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                textAlign = TextAlign.Center,
            )
        }
        Spacer(Modifier.height(16.dp))
        Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) { actions() }
    }
}

/**
 * Over every screen: "No internet connection" while offline, "Back
 * online" for a moment when it returns.
 */
@Composable
fun ConnectionBanner() {
    val online by App.instance.network.online.collectAsState()
    var wasOffline by remember { mutableStateOf(false) }
    var backOnline by remember { mutableStateOf(false) }
    LaunchedEffect(online) {
        if (!online) {
            wasOffline = true
        } else if (wasOffline) {
            wasOffline = false
            backOnline = true
            delay(2_500)
            backOnline = false
        }
    }
    AnimatedVisibility(!online || backOnline, enter = expandVertically(), exit = shrinkVertically()) {
        Row(
            Modifier
                .fillMaxWidth()
                .background(if (online) Color(0xFF1E6B34) else Color(0xFF3A3A3C))
                .statusBarsPadding()
                .padding(horizontal = 16.dp, vertical = 8.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Icon(if (online) Icons.Default.Wifi else Icons.Default.CloudOff, null, Modifier.size(18.dp), tint = Color.White)
            Spacer(Modifier.width(10.dp))
            Box(Modifier.weight(1f)) {
                Text(
                    stringResource(if (online) R.string.back_online else R.string.offline_banner),
                    color = Color.White,
                    style = MaterialTheme.typography.bodySmall,
                )
            }
        }
    }
}

private const val SLOW_HINT_MS = 6_000L

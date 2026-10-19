package dev.ytmplayer.android

import android.content.Context
import android.net.ConnectivityManager
import android.net.Network as AndroidNetwork
import android.net.NetworkCapabilities
import android.os.Handler
import android.os.Looper
import dev.ytmplayer.core.YTMError
import dev.ytmplayer.core.isNetworkFailure
import dev.ytmplayer.core.isTimeout
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow

/**
 * Whether the phone has a working internet connection (a network Android
 * validated), as a flow the UI and the player watch: banners, retries
 * when the connection comes back, the player waiting instead of skipping.
 */
class Network(context: Context) {
    private val connectivity = context.getSystemService(ConnectivityManager::class.java)
    private val main = Handler(Looper.getMainLooper())
    private val _online = MutableStateFlow(check())
    val online: StateFlow<Boolean> = _online

    init {
        // The callback's own arguments, not a fresh query: right after
        // onLost the lost network can still be the "active" one.
        connectivity.registerDefaultNetworkCallback(object : ConnectivityManager.NetworkCallback() {
            override fun onCapabilitiesChanged(network: AndroidNetwork, caps: NetworkCapabilities) = set(usable(caps))
            override fun onLost(network: AndroidNetwork) = set(false)
            override fun onUnavailable() = set(false)
        })
    }

    private fun set(online: Boolean) {
        main.post { _online.value = online }
    }

    private fun check(): Boolean =
        connectivity.getNetworkCapabilities(connectivity.activeNetwork)?.let(::usable) ?: false

    private fun usable(caps: NetworkCapabilities): Boolean =
        caps.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET) &&
            caps.hasCapability(NetworkCapabilities.NET_CAPABILITY_VALIDATED)
}

/** What went wrong, in the user's terms. */
enum class Problem { Offline, Slow, Server, SignIn, Other }

fun Throwable.problem(online: Boolean): Problem {
    val cause = rootYtmError() ?: this
    return when {
        !online -> Problem.Offline
        cause is YTMError.SignInRequired -> Problem.SignIn
        cause.isTimeout() -> Problem.Slow
        cause.isNetworkFailure() -> Problem.Offline
        cause is YTMError.Http && cause.status >= 500 -> Problem.Server
        else -> Problem.Other
    }
}

/** The YouTube error inside wrappers (ExoPlayer wraps what its data source throws). */
fun Throwable.rootYtmError(): Throwable? {
    var e: Throwable? = this
    while (e != null) {
        if (e is YTMError) return e
        if (e.cause === e) break
        e = e.cause
    }
    return null
}

/** One line for a status message: "No internet connection", the reason YouTube gave… */
fun Throwable.describe(context: Context, online: Boolean): String {
    val inner = rootYtmError() ?: this
    return when (problem(online)) {
        Problem.Offline -> context.getString(R.string.error_offline)
        Problem.Slow -> context.getString(R.string.error_slow)
        Problem.Server -> context.getString(R.string.error_server)
        Problem.SignIn -> inner.message ?: context.getString(R.string.error_sign_in)
        Problem.Other -> inner.message ?: inner.javaClass.simpleName
    }
}

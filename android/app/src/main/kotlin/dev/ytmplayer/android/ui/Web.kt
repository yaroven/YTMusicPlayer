package dev.ytmplayer.android.ui

import android.annotation.SuppressLint
import android.graphics.Color as AndroidColor
import android.webkit.CookieManager
import android.webkit.JavascriptInterface
import android.webkit.WebSettings
import android.webkit.WebView
import android.webkit.WebViewClient
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.viewinterop.AndroidView
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.CloudOff
import androidx.webkit.WebViewCompat
import androidx.webkit.WebViewFeature
import dev.ytmplayer.android.App
import dev.ytmplayer.android.BuildConfig
import dev.ytmplayer.android.R
import dev.ytmplayer.core.Auth

/**
 * The song's clip, in place of the cover — YouTube Music's song / video
 * switch. YouTube's watch page in a WebView, stripped down to the player by
 * `PAGE_SCRIPT` (the desktop player's and the iPhone app's script); the
 * page reports where the video is, so the audio continues there.
 */
@SuppressLint("SetJavaScriptEnabled", "JavascriptInterface")
@Composable
fun VideoPane(videoId: String, start: Double, progress: (Double) -> Unit) {
    val online by App.instance.network.online.collectAsState()
    if (!online) {
        Message(Icons.Default.CloudOff, stringResource(R.string.offline_title), stringResource(R.string.video_offline))
        return
    }
    Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
        AndroidView(
            factory = { context ->
                WebView(context).apply {
                    setBackgroundColor(AndroidColor.BLACK)
                    settings.javaScriptEnabled = true
                    settings.domStorageEnabled = true
                    settings.mediaPlaybackRequiresUserGesture = false
                    // The desktop page: the mobile one (m.youtube.com) is laid
                    // out differently and the script wouldn't strip it.
                    settings.userAgentString = DESKTOP_UA
                    addJavascriptInterface(
                        object {
                            @JavascriptInterface
                            fun postMessage(message: String) {
                                if (BuildConfig.DEBUG) android.util.Log.d("ytm-video", message)
                                message.toDoubleOrNull()?.takeIf { it.isFinite() }?.let { at -> post { progress(at) } }
                            }
                        },
                        "ytmIpc",
                    )
                    if (WebViewFeature.isFeatureSupported(WebViewFeature.DOCUMENT_START_SCRIPT)) {
                        WebViewCompat.addDocumentStartJavaScript(this, PAGE_SCRIPT, setOf("https://www.youtube.com"))
                        webViewClient = WebViewClient()
                    } else {
                        webViewClient = object : WebViewClient() {
                            override fun onPageFinished(view: WebView, url: String?) {
                                view.evaluateJavascript(PAGE_SCRIPT, null)
                            }
                        }
                    }
                    loadUrl("https://www.youtube.com/watch?v=$videoId&t=${start.toInt()}s")
                }
            },
            onRelease = { it.destroy() },
            modifier = Modifier.fillMaxWidth().aspectRatio(16f / 9f).clip(RoundedCornerShape(10.dp)).background(Color.Black),
        )
    }
}

/** Dark theme, nothing but the player, the player over the whole view, the position every half second. */
private const val PAGE_SCRIPT = """
(function () {
    if (window.ytmInjected) return; window.ytmInjected = true;
    window.ipc = { postMessage: function (m) { ytmIpc.postMessage(m); } };
    function setup() {
        document.documentElement.setAttribute('dark', 'true');
        var css = document.createElement('style');
        css.textContent =
            'html,body{background:#000!important;margin:0!important;overflow:hidden!important}'
            + 'ytd-masthead,#masthead-container,#secondary,#below,ytd-comments,#chat,'
            + 'tp-yt-app-drawer,#related,ytd-watch-metadata,#panels,ytd-popup-container'
            + '{display:none!important}'
            + 'ytd-app,#content,#page-manager,ytd-watch-flexy,#columns,#primary,#primary-inner,'
            + '#player,#player-container-outer,#player-container-inner,#movie_player'
            + '{background:#000!important}'
            + 'ytd-watch-flexy #primary,#player-container-outer,#player-container-inner'
            + '{padding:0!important;margin:0!important;max-width:100vw!important}'
            + '#player,#movie_player,.html5-video-player,.html5-video-container'
            + '{width:100vw!important;height:100vh!important;left:0!important;top:0!important}'
            + 'video{width:100%!important;height:100%!important;left:0!important;top:0!important;'
            + 'object-fit:contain!important}';
        (document.head || document.documentElement).appendChild(css);
    }
    if (document.documentElement) setup(); else document.addEventListener('DOMContentLoaded', setup);
    setInterval(function () {
        var v = document.querySelector('video');
        if (v) { window.ipc.postMessage(String(v.currentTime)); }
    }, 500);
})();
"""

private const val DESKTOP_UA =
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36"

/**
 * Google's own sign-in page in a WebView (the password goes only to
 * Google). Once youtube.com has a session, its cookies are taken and the
 * WebView's cookies are thrown away.
 */
@SuppressLint("SetJavaScriptEnabled")
@Composable
fun SignInDialog(onDismiss: () -> Unit) {
    val account = App.instance.account
    val online by App.instance.network.online.collectAsState()
    DisposableEffect(Unit) {
        CookieManager.getInstance().removeAllCookies(null)
        onDispose { CookieManager.getInstance().removeAllCookies(null) }
    }
    Dialog(onDismiss, DialogProperties(usePlatformDefaultWidth = false, decorFitsSystemWindows = false)) {
        Column(Modifier.fillMaxSize().background(Color.Black).statusBarsPadding().navigationBarsPadding()) {
            Box(Modifier.fillMaxWidth()) {
                TextButton(onClick = onDismiss) { Text(stringResource(R.string.cancel)) }
                Text(stringResource(R.string.google_account), Modifier.align(Alignment.Center))
            }
            if (!online) {
                Message(Icons.Default.CloudOff, stringResource(R.string.offline_title), stringResource(R.string.sign_in_offline))
                return@Column
            }
            AndroidView(
                factory = { context ->
                    WebView(context).apply {
                        settings.javaScriptEnabled = true
                        settings.domStorageEnabled = true
                        // Google refuses sign-in in what looks like an embedded web view.
                        settings.userAgentString = WebSettings.getDefaultUserAgent(context)
                            .replace("; wv", "").replace(Regex("Version/\\S+ "), "")
                        CookieManager.getInstance().setAcceptCookie(true)
                        webViewClient = object : WebViewClient() {
                            private var finished = false

                            override fun onPageFinished(view: WebView, url: String?) {
                                if (finished || url == null || !android.net.Uri.parse(url).host.orEmpty().endsWith("youtube.com")) return
                                val cookies = parseCookies(CookieManager.getInstance().getCookie("https://www.youtube.com"))
                                if (!Auth.isSignedIn(cookies)) return
                                finished = true
                                account.signIn(cookies)
                                onDismiss()
                            }
                        }
                        loadUrl(SIGN_IN_URL)
                    }
                },
                onRelease = { it.destroy() },
                modifier = Modifier.fillMaxSize(),
            )
        }
    }
}

private fun parseCookies(header: String?): Map<String, String> = header.orEmpty().split(';').mapNotNull {
    val eq = it.indexOf('=')
    if (eq <= 0) null else it.substring(0, eq).trim() to it.substring(eq + 1).trim()
}.toMap()

/** After sign-in Google sends the browser on to YouTube Music. */
private const val SIGN_IN_URL =
    "https://accounts.google.com/ServiceLogin?service=youtube&uilel=3&passive=true&continue=" +
        "https%3A%2F%2Fwww.youtube.com%2Fsignin%3Faction_handle_signin%3Dtrue%26app%3Dm%26next%3Dhttps%253A%252F%252Fmusic.youtube.com%252F"

private val Int.dp get() = androidx.compose.ui.unit.Dp(this.toFloat())

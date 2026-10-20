package dev.yarek.music

import android.annotation.SuppressLint
import android.content.Context
import android.webkit.JavascriptInterface
import android.webkit.WebView
import android.webkit.WebViewClient
import dev.yarek.core.JsEngine
import dev.yarek.core.YTMError
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeout
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException

/**
 * yt-dlp's challenge solver in a throwaway, off-screen WebView (the
 * iPhone app uses JavaScriptCore): one per call, destroyed after, so the
 * 2.5 MB player script doesn't stay in memory. Only signed-in playback of
 * age-restricted songs needs it.
 */
class WebViewJs(private val context: Context) : JsEngine {
    @SuppressLint("SetJavaScriptEnabled", "JavascriptInterface")
    override suspend fun solve(script: String, input: String): String = withTimeout(30_000) {
        withContext(Dispatchers.Main) {
            val web = WebView(context)
            try {
                suspendCancellableCoroutine { cont ->
                    val bridge = object {
                        @JavascriptInterface
                        fun input(): String = input

                        @JavascriptInterface
                        fun done(result: String) {
                            if (cont.isActive) cont.resume(result)
                        }

                        @JavascriptInterface
                        fun fail(message: String) {
                            if (cont.isActive) cont.resumeWithException(YTMError.Unplayable("JavaScript solver: $message"))
                        }
                    }
                    web.settings.javaScriptEnabled = true
                    web.addJavascriptInterface(bridge, "ytm")
                    web.webViewClient = object : WebViewClient() {
                        override fun onPageFinished(view: WebView, url: String?) {
                            view.evaluateJavascript(
                                "$script\n;try{ytm.done(JSON.stringify(jsc(JSON.parse(ytm.input()))))}" +
                                    "catch(e){ytm.fail(String(e))}",
                                null,
                            )
                        }
                    }
                    web.loadDataWithBaseURL(null, "<html></html>", "text/html", "utf-8", null)
                }
            } finally {
                web.destroy()
            }
        }
    }
}

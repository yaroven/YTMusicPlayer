package dev.ytmplayer.android

import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.pm.PackageInstaller
import android.os.Build
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import dev.ytmplayer.core.Http
import dev.ytmplayer.core.YTMError
import dev.ytmplayer.core.arr
import dev.ytmplayer.core.objects
import dev.ytmplayer.core.str
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.json.JSONObject
import java.security.MessageDigest

/** A newer release: its version and its APK. */
data class Update(val version: String, val apk: String, val sums: String?)

/**
 * Self-update, like the desktop player's: GitHub's "latest release" (the
 * repository is public, no token), at most once a day; the APK is checked
 * against the release's SHA256SUMS and handed to Android's package
 * installer, which asks once to allow installs from this app, then shows
 * its own "Update" dialog. Releases are signed with the same key, so the
 * update keeps the library, downloads and sign-in.
 */
class Updater(private val context: Context) {
    var available by mutableStateOf<Update?>(null)
        private set
    var installing by mutableStateOf(false)
        private set
    var error by mutableStateOf<String?>(null)
    /** "Up to date" after a check the user asked for. */
    var upToDate by mutableStateOf(false)
        private set

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private val prefs = context.getSharedPreferences("update", Context.MODE_PRIVATE)

    /** At start: once a day, quietly. */
    fun checkDaily() {
        val last = prefs.getLong("checked", 0)
        if (System.currentTimeMillis() - last < DAY_MS) {
            prefs.getString("version", null)?.let { v ->
                if (newer(v, BuildConfig.VERSION_NAME)) {
                    available = Update(v, prefs.getString("apk", null) ?: return, prefs.getString("sums", null))
                }
            }
            return
        }
        check(quiet = true)
    }

    fun check(quiet: Boolean = false) {
        scope.launch {
            upToDate = false
            try {
                val update = latest()
                prefs.edit().putLong("checked", System.currentTimeMillis())
                    .putString("version", update?.version).putString("apk", update?.apk).putString("sums", update?.sums).apply()
                available = update?.takeIf { newer(it.version, BuildConfig.VERSION_NAME) }
                upToDate = available == null && !quiet
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                if (!quiet) error = context.getString(R.string.update_failed, e.describe(context, App.instance.network.online.value))
            }
        }
    }

    private suspend fun latest(): Update? {
        val r = Http.get(LATEST, mapOf("Accept" to "application/vnd.github+json", "User-Agent" to "ytm-player"))
        if (!r.ok) throw YTMError.Http("GitHub", r.status)
        val release = JSONObject(r.text)
        val version = release.str("tag_name")?.removePrefix("v") ?: return null
        val assets = release.arr("assets")?.objects().orEmpty()
        val apk = assets.firstOrNull { it.optString("name").endsWith(".apk") }?.str("browser_download_url") ?: return null
        val sums = assets.firstOrNull { it.optString("name") == "SHA256SUMS" }?.str("browser_download_url")
        return Update(version, apk, sums)
    }

    /** Downloads, checks and installs `available`. */
    fun install() {
        val update = available ?: return
        if (installing) return
        installing = true
        scope.launch {
            try {
                val apk = Http.send(update.apk, headers = mapOf("User-Agent" to "ytm-player"), readTimeoutMs = 60_000)
                if (!apk.ok) throw YTMError.Http("GitHub", apk.status)
                update.sums?.let { url ->
                    val name = update.apk.substringAfterLast('/')
                    val want = Http.get(url, mapOf("User-Agent" to "ytm-player")).text.lines()
                        .firstOrNull { it.trim().endsWith(name) }?.substringBefore(' ')?.trim()
                    val have = MessageDigest.getInstance("SHA-256").digest(apk.body).joinToString("") { "%02x".format(it) }
                    if (want != null && want != have) throw YTMError.Unplayable(context.getString(R.string.update_bad_checksum))
                }
                withContext(Dispatchers.IO) { commit(apk.body) }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                error = context.getString(R.string.update_failed, e.describe(context, App.instance.network.online.value))
            } finally {
                installing = false
            }
        }
    }

    private fun commit(bytes: ByteArray) {
        val installer = context.packageManager.packageInstaller
        val params = PackageInstaller.SessionParams(PackageInstaller.SessionParams.MODE_FULL_INSTALL)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            params.setRequireUserAction(PackageInstaller.SessionParams.USER_ACTION_NOT_REQUIRED)
        }
        val id = installer.createSession(params)
        installer.openSession(id).use { session ->
            session.openWrite("ytm-player.apk", 0, bytes.size.toLong()).use { out ->
                out.write(bytes)
                session.fsync(out)
            }
            val flags = PendingIntent.FLAG_UPDATE_CURRENT or
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) PendingIntent.FLAG_MUTABLE else 0
            val done = PendingIntent.getBroadcast(context, id, Intent(context, InstallResult::class.java), flags)
            session.commit(done.intentSender)
        }
    }

    companion object {
        const val LATEST = "https://api.github.com/repos/yaroven/YTMusicPlayer/releases/latest"
        private const val DAY_MS = 24 * 3600 * 1000L

        /** "0.10.1" > "0.9.6", part by part. */
        fun newer(candidate: String, current: String): Boolean {
            val a = candidate.split('.').map { it.toIntOrNull() ?: 0 }
            val b = current.split('.').map { it.toIntOrNull() ?: 0 }
            for (i in 0 until maxOf(a.size, b.size)) {
                val x = a.getOrElse(i) { 0 }
                val y = b.getOrElse(i) { 0 }
                if (x != y) return x > y
            }
            return false
        }
    }
}

/**
 * The installer's answer: it needs the user (its own confirmation, or the
 * one-time "allow installs from this app"), or it failed.
 */
class InstallResult : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        when (intent.getIntExtra(PackageInstaller.EXTRA_STATUS, PackageInstaller.STATUS_FAILURE)) {
            PackageInstaller.STATUS_PENDING_USER_ACTION -> {
                @Suppress("DEPRECATION")
                val confirm = if (Build.VERSION.SDK_INT >= 33) {
                    intent.getParcelableExtra(Intent.EXTRA_INTENT, Intent::class.java)
                } else {
                    intent.getParcelableExtra(Intent.EXTRA_INTENT)
                }
                confirm?.let { context.startActivity(it.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) }
            }
            PackageInstaller.STATUS_SUCCESS -> {}
            else -> {
                val message = intent.getStringExtra(PackageInstaller.EXTRA_STATUS_MESSAGE)
                App.instance.updater.error = context.getString(R.string.update_failed, message ?: "")
            }
        }
    }
}

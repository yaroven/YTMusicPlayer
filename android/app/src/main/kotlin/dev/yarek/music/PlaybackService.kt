package dev.yarek.music

import android.app.PendingIntent
import android.content.Intent
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.net.Uri
import androidx.annotation.OptIn
import androidx.media3.common.util.BitmapLoader
import androidx.media3.common.util.UnstableApi
import androidx.media3.session.MediaSession
import androidx.media3.session.MediaSessionService
import com.google.common.util.concurrent.ListenableFuture
import com.google.common.util.concurrent.MoreExecutors
import java.net.HttpURLConnection
import java.net.URL
import java.util.concurrent.Executors

/**
 * Puts the player's media session in front of the system: notification,
 * lock screen, Bluetooth and headset buttons, and the foreground service
 * that keeps the music going with the app in the background.
 */
@OptIn(UnstableApi::class)
class PlaybackService : MediaSessionService() {
    private var session: MediaSession? = null

    override fun onCreate() {
        super.onCreate()
        val open = PendingIntent.getActivity(
            this, 0, Intent(this, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        session = MediaSession.Builder(this, App.instance.player.exo)
            .setSessionActivity(open)
            .setBitmapLoader(SquareArtLoader())
            .build()
            .also { addSession(it) }
    }

    override fun onGetSession(controllerInfo: MediaSession.ControllerInfo): MediaSession? = session

    override fun onTaskRemoved(rootIntent: Intent?) {
        // Swiped away while paused: nothing left to keep running for.
        if (!App.instance.player.exo.playWhenReady) stopSelf()
    }

    override fun onDestroy() {
        // The player belongs to the app; only the session goes.
        session?.release()
        session = null
        super.onDestroy()
    }
}

/**
 * Notification / lock screen art: the centre square of the video
 * thumbnail (maxresdefault, else hqdefault with its letterbox bars cut).
 */
@OptIn(UnstableApi::class)
private class SquareArtLoader : BitmapLoader {
    private val executor = MoreExecutors.listeningDecorator(Executors.newSingleThreadExecutor())

    override fun supportsMimeType(mimeType: String): Boolean = mimeType.startsWith("image/")

    override fun decodeBitmap(data: ByteArray): ListenableFuture<Bitmap> =
        executor.submit<Bitmap> { square(BitmapFactory.decodeByteArray(data, 0, data.size) ?: error("not an image"), 512) }

    override fun loadBitmap(uri: Uri): ListenableFuture<Bitmap> = executor.submit<Bitmap> {
        // A downloaded song's cover is on the phone (offline too).
        if (uri.scheme == "file") return@submit BitmapFactory.decodeFile(uri.path) ?: error("no art")
        val urls = listOf(uri.toString(), uri.toString().replace("maxresdefault", "hqdefault")).distinct()
        urls.firstNotNullOfOrNull(::fetch)?.let { square(it, 512) } ?: error("no art")
    }

    private fun fetch(url: String): Bitmap? = runCatching {
        val c = URL(url).openConnection() as HttpURLConnection
        c.connectTimeout = 10_000
        c.readTimeout = 15_000
        try {
            if (c.responseCode != 200) null else c.inputStream.use { BitmapFactory.decodeStream(it) }
        } finally {
            c.disconnect()
        }
    }.getOrNull()
}

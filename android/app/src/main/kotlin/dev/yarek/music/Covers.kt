package dev.yarek.music

import android.content.Context
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import dev.yarek.core.Track
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.io.File
import java.net.HttpURLConnection
import java.net.URL
import java.util.Collections

/**
 * Covers of downloaded songs, kept next to them so the notification, the
 * lock screen and the lists show them offline: files/covers/<video id>.jpg,
 * the centre square at up to 544 px.
 */
object Covers {
    private lateinit var dir: File

    /** Which covers are on disk (read once, then kept up to date). */
    private val saved: MutableSet<String> = Collections.synchronizedSet(HashSet())

    fun init(context: Context) {
        dir = File(context.filesDir, "covers").apply { mkdirs() }
        dir.list()?.filter { it.endsWith(".jpg") }?.forEach { saved += it.removeSuffix(".jpg") }
    }

    /** The cover's file, when this song's cover is on the phone. */
    fun local(videoId: String): File? = if (videoId in saved) File(dir, "$videoId.jpg") else null

    /** Fetches and keeps the song's cover: its album art when YouTube Music has one, else the clip's frame (maxresdefault, then hqdefault). */
    suspend fun fetch(track: Track) {
        if (local(track.videoId) != null) return
        val album = Pairs.pair(track, findClip = false)?.cover?.let { dev.yarek.music.ui.resized(it, 1200) }
        withContext(Dispatchers.IO) { keep(track, listOfNotNull(album, track.largeThumbnailUrl, "https://i.ytimg.com/vi/${track.videoId}/hqdefault.jpg")) }
    }

    private fun keep(track: Track, urls: List<String>) {
        val image = urls.firstNotNullOfOrNull(::download) ?: return
        val square = square(image, 544)
        val file = File(dir, "${track.videoId}.jpg")
        val part = File(dir, "${track.videoId}.jpg.part")
        runCatching {
            part.outputStream().use { square.compress(Bitmap.CompressFormat.JPEG, 85, it) }
            part.renameTo(file)
            saved += track.videoId
        }
        part.delete()
    }

    fun remove(videoId: String) {
        saved -= videoId
        File(dir, "$videoId.jpg").delete()
    }

    private fun download(url: String): Bitmap? = runCatching {
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

/** The centre square of a (letterboxed) video thumbnail, at most `max` px. */
fun square(image: Bitmap, max: Int): Bitmap {
    val w = image.width
    val h = image.height
    // 4:3 with bars: the picture is the middle 16:9 band.
    val band = if (kotlin.math.abs(w.toDouble() / h - 4.0 / 3.0) < 0.01) w * 9 / 16 else h
    val side = minOf(w, band)
    val square = Bitmap.createBitmap(image, (w - side) / 2, (h - side) / 2, side, side)
    val scaled = if (side > max) Bitmap.createScaledBitmap(square, max, max, true) else square
    if (scaled !== square) square.recycle()
    return scaled
}

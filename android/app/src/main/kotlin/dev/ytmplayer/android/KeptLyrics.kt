package dev.ytmplayer.android

import android.content.Context
import dev.ytmplayer.core.Lyrics
import dev.ytmplayer.core.Track
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import org.json.JSONObject
import java.io.File

/**
 * Lyrics kept on the phone (files/lyrics/<video id>.json): every downloaded
 * song's, so they show offline, and the last [RECENT] others, so a song
 * played again doesn't ask LRCLIB again.
 */
object KeptLyrics {
    private const val RECENT = 50
    private lateinit var dir: File

    fun init(context: Context) {
        dir = File(context.filesDir, "lyrics").apply { mkdirs() }
    }

    /** Video ids are 11 URL-safe characters; nothing else names a file. */
    private fun file(videoId: String): File? =
        if (videoId.length == 11 && videoId.all { it.isLetterOrDigit() && it.code < 128 || it == '-' || it == '_' }) {
            File(dir, "$videoId.json")
        } else {
            null
        }

    fun get(videoId: String): Lyrics? =
        file(videoId)?.takeIf { it.exists() }?.let { f -> runCatching { Lyrics.fromJson(JSONObject(f.readText())) }.getOrNull() }

    fun contains(videoId: String): Boolean = file(videoId)?.exists() == true

    /** Saves `lyrics`, then drops the oldest that `keep` (the downloads) doesn't name, beyond [RECENT]. */
    fun put(videoId: String, lyrics: Lyrics, keep: Set<String>) {
        val file = file(videoId) ?: return
        val part = File(dir, "$videoId.json.part")
        runCatching {
            part.writeText(lyrics.toJson().toString())
            part.renameTo(file)
        }
        part.delete()
        dir.listFiles { f -> f.name.endsWith(".json") && f.nameWithoutExtension !in keep }
            ?.sortedByDescending { it.lastModified() }
            ?.drop(RECENT)
            ?.forEach { it.delete() }
    }

    fun remove(videoId: String) {
        file(videoId)?.delete()
    }

    /** Kept lyrics, else found (and kept) — what the lyrics pane shows. */
    suspend fun find(track: Track, duration: Double?, keep: Set<String>): Lyrics? = withContext(Dispatchers.IO) {
        get(track.videoId)?.let { return@withContext it }
        val found = App.instance.lyrics.find(track, duration)
        if (found != null) put(track.videoId, found, keep)
        found
    }
}

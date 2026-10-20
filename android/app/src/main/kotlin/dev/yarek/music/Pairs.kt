package dev.yarek.music

import dev.yarek.core.MediaPair
import dev.yarek.core.Track
import dev.yarek.core.mediaPair
import java.util.concurrent.ConcurrentHashMap

/**
 * Songs and their clips found so far (YouTube Music's "Song" / "Video",
 * [dev.yarek.core.mediaPair]): what plays as a track's sound (the studio
 * song, also for a clip) and which clip "Video" shows. In memory, a few
 * hundred.
 */
object Pairs {
    private class Known(val pair: MediaPair, val complete: Boolean)
    private val known = ConcurrentHashMap<String, Known>()

    /** The id whose sound plays for [track]: its studio song (one request the first time); its own id when the catalog can't tell. */
    suspend fun audioId(track: Track): String = pair(track, findClip = false)?.song ?: track.videoId

    /** The audio id already found for [videoId], if any. */
    fun cachedAudioId(videoId: String): String = known[videoId]?.pair?.song ?: videoId

    /** The song and its clip; [findClip] also looks for the clip of a song. */
    suspend fun pair(track: Track, findClip: Boolean): MediaPair? {
        known[track.videoId]?.let { if (it.complete || !findClip || it.pair.video != null) return it.pair }
        val pair = runCatching {
            App.instance.catalog.mediaPair(track.videoId, track.title, track.artist, findClip)
        }.getOrNull() ?: return null
        if (known.size >= 500) known.clear()
        known[track.videoId] = Known(pair, findClip)
        return pair
    }
}

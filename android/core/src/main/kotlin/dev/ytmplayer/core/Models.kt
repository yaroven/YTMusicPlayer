package dev.ytmplayer.core

import org.json.JSONObject

/** A song. */
data class Track(
    val videoId: String,
    val title: String,
    val artist: String,
    val durationSecs: Int? = null,
    /** YouTube Music's "E" badge: explicit lyrics (18+). */
    val explicit: Boolean = false,
) {
    /** YouTube's 16:9 thumbnail of the video; views crop the centre square. */
    val thumbnailUrl: String get() = "https://i.ytimg.com/vi/$videoId/mqdefault.jpg"

    /** The sharpest thumbnail (1280x720); missing for some videos. */
    val largeThumbnailUrl: String get() = "https://i.ytimg.com/vi/$videoId/maxresdefault.jpg"

    /** Always there: 480x360 with letterbox bars. */
    val fallbackThumbnailUrl: String get() = "https://i.ytimg.com/vi/$videoId/hqdefault.jpg"

    fun toJson(): JSONObject = JSONObject().apply {
        put("videoId", videoId)
        put("title", title)
        put("artist", artist)
        durationSecs?.let { put("durationSecs", it) }
        if (explicit) put("explicit", true)
    }

    companion object {
        fun fromJson(o: JSONObject): Track? {
            val id = o.str("videoId") ?: return null
            return Track(
                videoId = id,
                title = o.optString("title"),
                artist = o.optString("artist"),
                durationSecs = (o.opt("durationSecs") as? Number)?.toInt(),
                explicit = o.optBoolean("explicit"),
            )
        }
    }
}

/** "ABBA - Topic" (auto-generated channels) → "ABBA". */
fun cleanArtist(channel: String): String = channel.removeSuffix(" - Topic")

enum class SearchKind(val label: String, internal val params: String) {
    Songs("Songs", "EgWKAQIIAWoMEA4QChADEAQQCRAF"),
    Albums("Albums", "EgWKAQIYAWoMEA4QChADEAQQCRAF"),
    Artists("Artists", "EgWKAQIgAWoMEA4QChADEAQQCRAF"),
    Playlists("Playlists", "EgWKAQIoAWoMEA4QChADEAQQCRAF"),
}

/** A song's rating in the account. */
enum class Rating { Like, Dislike, None }

enum class ItemKind { Song, Album, Artist, Playlist }

/** A search result or a card on a page. */
data class Item(
    val kind: ItemKind,
    /** Video id for songs, browse id (`MPREb_…`, `UC…`, `VL…`) otherwise. */
    val browseId: String,
    val title: String,
    /** "Album • ABBA • 1992", "Artist • 290M monthly audience", … */
    val subtitle: String,
    val thumbnail: String? = null,
    /** Songs only. */
    val track: Track? = null,
    val explicit: Boolean = false,
) {
    val id: String get() = "${kind.name}:$browseId"
}

/** A titled row of cards. */
data class Shelf(val title: String, val items: List<Item>) {
    val id: String get() = title + (items.firstOrNull()?.id ?: "")
}

/** An album, artist or playlist page. */
data class Page(
    val kind: ItemKind,
    val id: String,
    val title: String,
    val subtitle: String,
    val thumbnail: String?,
    val tracks: List<Track>,
    val shelves: List<Shelf>,
    /** More of `tracks` to fetch (`Catalog.moreTracks`): long playlists. */
    val continuation: String? = null,
    /** An artist's "Top songs" playlist, with all of them (the page lists 5). */
    val allSongsId: String? = null,
)

/** One lyrics line; `at` is when it starts (synced lyrics only). */
data class LyricLine(
    val at: Double?,
    val text: String,
    /** Word start times when the lyrics have them (Enhanced LRC). */
    val wordTimes: List<Double> = emptyList(),
) {
    /** No words: an instrumental break ("", "♪"). */
    val instrumental: Boolean get() = text.all { it.isWhitespace() || it in "♪♫♬…•" }

    /** The words, each with its trailing space. */
    val words: List<String>
        get() {
            val out = ArrayList<String>()
            val current = StringBuilder()
            var inSpace = false
            for (c in text) {
                if (c.isWhitespace()) {
                    inSpace = true
                } else if (inSpace) {
                    out.add(current.toString())
                    current.clear()
                    inSpace = false
                }
                current.append(c)
            }
            if (current.isNotEmpty()) out.add(current.toString())
            return out
        }
}

/**
 * Karaoke state: the line being sung and how many words (or, on an
 * instrumental line, dots 0..3) are lit. Line -1 is the intro.
 */
data class LyricsProgress(val line: Int, val words: Int, val chars: Int = words)

data class Lyrics(val lines: List<LyricLine>, val source: String) {
    val synced: Boolean get() = lines.firstOrNull()?.at != null

    /** Index of the line sung at `position` (synced lyrics). */
    fun currentLine(position: Double): Int? {
        if (!synced) return null
        val i = lines.indexOfLast { (it.at ?: Double.POSITIVE_INFINITY) <= position }
        return if (i >= 0) i else null
    }

    val hasIntro: Boolean get() = synced && (lines.firstOrNull()?.at ?: 0.0) >= INTRO_GAP

    /**
     * When each word of line `i` starts: the lyrics' own times, else spread
     * by length over the time the line is sung (as the desktop player's
     * `Lyrics::word_times`).
     */
    fun wordTimes(i: Int): List<Double> {
        val line = lines.getOrNull(i) ?: return emptyList()
        val words = line.words
        if (line.wordTimes.size == words.size && words.isNotEmpty()) return line.wordTimes
        val start = line.at ?: return emptyList()
        val chars = words.map { maxOf(1, it.trim().length) }
        val total = chars.sum()
        var span = PER_CHAR * total + LINE_EXTRA
        val next = lines.getOrNull(i + 1)?.at
        if (next != null) {
            val gap = next - start
            span = maxOf(minOf(span, gap - 0.2), minOf(gap, 0.5))
        }
        var before = 0
        return chars.map { c ->
            val t = start + span * before / maxOf(total, 1)
            before += c
            t
        }
    }

    fun progress(position: Double): LyricsProgress? {
        if (!synced) return null
        val first = lines.first().at
        if (hasIntro && first != null && position < first) {
            return LyricsProgress(-1, minOf(3, (position / maxOf(first, 0.1) * 4).toInt()))
        }
        val line = currentLine(position) ?: return null
        val current = lines[line]
        if (current.instrumental) {
            val start = current.at ?: 0.0
            val end = lines.getOrNull(line + 1)?.at ?: (start + 6)
            return LyricsProgress(line, minOf(3, ((position - start) / maxOf(end - start, 0.1) * 4).toInt()))
        }
        val words = current.words
        val sung = wordTimes(line).count { it <= position }
        // Every word that has started lights up whole (not letter by letter).
        val chars = words.take(minOf(sung, words.size)).sumOf { it.length }
        return LyricsProgress(line, sung, chars)
    }

    companion object {
        /** Before the first line, a gap this long shows as an instrumental intro. */
        const val INTRO_GAP = 4.0
        private const val PER_CHAR = 0.085
        private const val LINE_EXTRA = 0.4
    }
}

sealed class YTMError(message: String) : Exception(message) {
    class Http(what: String, val status: Int) : YTMError("$what: HTTP $status")
    class Unplayable(reason: String) : YTMError(reason)

    /** Age-restricted (or a bot check): needs a signed-in account. */
    class SignInRequired(reason: String) : YTMError(reason)
    object NoStream : YTMError("YouTube returned no audio stream for this song")
}

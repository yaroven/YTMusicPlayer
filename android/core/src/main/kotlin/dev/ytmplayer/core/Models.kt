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

/** How YouTube Music lays a shelf out. */
enum class ShelfStyle {
    /** Square cards in a row (albums, playlists, artists). */
    Cards,

    /** Song rows ("Quick picks"): a grid of small tiles, three rows high. */
    Grid,

    /** 16:9 cards (music videos). */
    Wide,
}

/** A titled row of cards. */
data class Shelf(
    val title: String,
    val items: List<Item>,
    val style: ShelfStyle = ShelfStyle.Cards,
    /** The small line above the title ("JANE DOE", "TIMELESS CLASSICS"). */
    val strapline: String = "",
    /** The round picture next to it (the account, an artist). */
    val straplineArt: String? = null,
) {
    val id: String get() = title + (items.firstOrNull()?.id ?: "")
}

/** A mood chip over Home ("Relax", "Workout"): Home again, filtered. */
data class HomeChip(val title: String, val params: String)

/** Home: its chips, its first shelves, and how to get more of them. */
data class HomeFeed(
    val chips: List<HomeChip>,
    val shelves: List<Shelf>,
    val continuation: String?,
    /** The visitor id YouTube gave; continuations need it back. */
    val visitor: String?,
)

/** Who is signed in. */
data class AccountInfo(val name: String, val photo: String?)

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
     * When each word of line `i` starts, when the lyrics time words
     * (Enhanced LRC); empty otherwise — guessed word times lit words out of
     * step with the singing, so the line lights up whole (as the desktop
     * player's `Lyrics::word_times`).
     */
    fun wordTimes(i: Int): List<Double> {
        val line = lines.getOrNull(i) ?: return emptyList()
        val words = line.words
        return if (line.wordTimes.size == words.size && words.isNotEmpty()) line.wordTimes else emptyList()
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
        val times = wordTimes(line)
        // Only lines are timed: the line lights up whole.
        if (times.isEmpty()) return LyricsProgress(line, words.size, current.text.length)
        val sung = times.count { it <= position }
        // Every word that has started lights up whole (not letter by letter).
        val chars = words.take(minOf(sung, words.size)).sumOf { it.length }
        return LyricsProgress(line, sung, chars)
    }

    companion object {
        /** Before the first line, a gap this long shows as an instrumental intro. */
        const val INTRO_GAP = 4.0

        /** Kept lyrics (see [toJson]); null when it isn't such JSON. */
        fun fromJson(o: JSONObject): Lyrics? = runCatching {
            val lines = o.getJSONArray("lines")
            Lyrics(
                (0 until lines.length()).map { i ->
                    val l = lines.getJSONObject(i)
                    val times = l.optJSONArray("words")
                    LyricLine(
                        if (l.has("at")) l.getDouble("at") else null,
                        l.getString("text"),
                        times?.let { t -> (0 until t.length()).map(t::getDouble) }.orEmpty(),
                    )
                },
                o.getString("source"),
            )
        }.getOrNull()
    }

    /** For keeping on disk (a downloaded song's lyrics, offline). */
    fun toJson(): JSONObject = JSONObject().apply {
        put("source", source)
        put(
            "lines",
            org.json.JSONArray().apply {
                lines.forEach { l ->
                    put(
                        JSONObject().apply {
                            l.at?.let { put("at", it) }
                            put("text", l.text)
                            if (l.wordTimes.isNotEmpty()) put("words", org.json.JSONArray(l.wordTimes))
                        },
                    )
                }
            },
        )
    }
}

sealed class YTMError(message: String) : Exception(message) {
    class Http(what: String, val status: Int) : YTMError("$what: HTTP $status")
    class Unplayable(reason: String) : YTMError(reason)

    /** Age-restricted (or a bot check): needs a signed-in account. */
    class SignInRequired(reason: String) : YTMError(reason)
    object NoStream : YTMError("YouTube returned no audio stream for this song")
}

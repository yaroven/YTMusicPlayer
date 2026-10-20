package dev.yarek.core

import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.delay
import org.json.JSONArray
import org.json.JSONObject
import java.io.IOException
import java.net.URLEncoder
import kotlin.math.abs

/** Synced lyrics from LRCLIB, else YouTube Music's plain text. Port of YTMKit's LyricsFinder.swift. */
class LyricsFinder(private val catalog: Catalog) {
    suspend fun find(track: Track, duration: Double?): Lyrics? {
        val lrc = try {
            lrclib(track, duration)
        } catch (e: CancellationException) {
            throw e
        } catch (_: Exception) {
            null
        }
        if (lrc != null && lrc.synced) return lrc
        return catalog.lyrics(track.videoId) ?: lrc
    }

    /** One LRCLIB request; null on 404. Retried on 5xx / 429 / network errors. */
    private suspend fun get(base: String, query: List<Pair<String, String>>): String? {
        val url = base + "?" + query.joinToString("&") { (k, v) -> "$k=" + URLEncoder.encode(v, "UTF-8") }
        var wait = 400L
        for (attempt in 1..3) {
            try {
                val r = Http.send(url, headers = mapOf("User-Agent" to "yarek-music (https://github.com/yaroven/YTMusicPlayer)"), readTimeoutMs = 10_000)
                if (r.status == 404) return null
                if (r.status == 200) return r.text
                if (attempt == 3 || !(r.status >= 500 || r.status == 429)) throw YTMError.Http("LRCLIB", r.status)
            } catch (e: IOException) {
                if (attempt == 3) throw e
            }
            delay(wait)
            wait *= 2
        }
        return null
    }

    private class Entry(val duration: Double?, val synced: String?, val plain: String?) {
        constructor(o: JSONObject) : this(
            (o.opt("duration") as? Number)?.toDouble(),
            o.opt("syncedLyrics") as? String,
            o.opt("plainLyrics") as? String,
        )
    }

    private suspend fun lrclib(track: Track, duration: Double?): Lyrics? {
        val (artist, title) = cleanTitle(track.artist, track.title)
        val query = listOf("track_name" to title, "artist_name" to artist)
        // The exact match first (LRCLIB caches it), then a search.
        val entries = ArrayList<Entry>()
        if (duration != null) {
            get(LRCLIB_GET, query + ("duration" to duration.toInt().toString()))?.let { entries += Entry(JSONObject(it)) }
        }
        if (entries.none { !it.synced.isNullOrEmpty() }) {
            get(LRCLIB_SEARCH, query)?.let { body -> entries += JSONArray(body).objects().map(::Entry) }
        }
        val fits = entries.filter { e -> duration == null || e.duration == null || abs(duration - e.duration) <= DURATION_SLACK }
        val synced = fits.firstNotNullOfOrNull { it.synced?.takeIf(String::isNotEmpty) }
        val lines = when {
            synced != null -> parseLrc(synced)
            else -> fits.firstNotNullOfOrNull { it.plain }?.split('\n')?.map { LyricLine(null, it) } ?: return null
        }
        return if (lines.isEmpty()) null else Lyrics(lines, "LRCLIB")
    }

    companion object {
        const val LRCLIB_SEARCH = "https://lrclib.net/api/search"
        const val LRCLIB_GET = "https://lrclib.net/api/get"

        /** LRCLIB entries whose length differs more than this are another cut. */
        const val DURATION_SLACK = 4.0
    }
}

internal fun lrcTime(stamp: String): Double? {
    val parts = stamp.split(':')
    if (parts.size != 2) return null
    val min = parts[0].toDoubleOrNull() ?: return null
    val sec = parts[1].toDoubleOrNull() ?: return null
    return min * 60 + sec
}

/**
 * `[mm:ss.xx] text` lines; untimed ones (metadata tags) are dropped.
 * Enhanced LRC word stamps (`<mm:ss.xx>word`) become word times.
 */
fun parseLrc(lrc: String): List<LyricLine> = lrc.split('\n').mapNotNull { line ->
    if (!line.startsWith("[")) return@mapNotNull null
    val close = line.indexOf(']')
    if (close < 0) return@mapNotNull null
    val at = lrcTime(line.substring(1, close)) ?: return@mapNotNull null
    val text = StringBuilder()
    val times = ArrayList<Double>()
    line.substring(close + 1).split('<').forEachIndexed { i, part ->
        val end = part.indexOf('>')
        val stamp = if (i > 0 && end >= 0) lrcTime(part.substring(0, end)) else null
        if (stamp != null) {
            val word = part.substring(end + 1)
            if (word.isNotBlank()) times += stamp
            text.append(word)
        } else {
            if (i > 0) text.append('<')
            text.append(part)
        }
    }
    LyricLine(at, text.toString().trim(), times)
}

/** "ABBA - Dancing Queen (Official Music Video)" by "ABBA" → ("ABBA", "Dancing Queen"). */
fun cleanTitle(artist: String, title: String): Pair<String, String> {
    var a = artist.trim()
    var t = title.trim()
    val dash = t.indexOf(" - ")
    if (dash >= 0) {
        val left = t.substring(0, dash)
        if (a.isEmpty() || left.lowercase().contains(a.lowercase()) || a.endsWith("VEVO")) {
            a = left.trim()
            t = t.substring(dash + 3).trim()
        }
    }
    // Drop bracketed decorations: (Official Video), [Lyrics], (Remastered 2009)…
    while (true) {
        val cuts = listOf('(' to ')', '[' to ']').mapNotNull { (open, close) ->
            val start = t.indexOf(open)
            if (start < 0) return@mapNotNull null
            val end = t.indexOf(close, start + 1)
            if (end < 0) null else start until end + 1
        }
        val cut = cuts.minByOrNull { it.first } ?: break
        t = t.removeRange(cut)
    }
    for (noise in listOf("ft.", "feat.")) {
        val i = t.lowercase().indexOf(noise)
        if (i >= 0) t = t.substring(0, i)
    }
    return a.trim() to t.trim()
}

package dev.yarek.core

/**
 * A song and its clip (YouTube Music's "Song" / "Video"); see
 * [Catalog.mediaPair]. Same rules as the desktop player's `MediaPair`.
 */
data class MediaPair(
    /** What plays as sound: the studio song (the clip's own sound only when no studio song was found). */
    val song: String,
    /** The clip "Video" shows, when there is one. */
    val video: String?,
    /**
     * Seconds the clip runs ahead of the song: clip time = song time +
     * offset. Guessed from the lengths (a longer clip has an intro), so it
     * can be a few seconds off when the extra is an outro instead.
     */
    val offset: Double,
    /**
     * The square album art (googleusercontent) YouTube Music shows for the
     * song, when it has one; null when only a frame of the clip exists.
     */
    val cover: String? = null,
)

/** YouTube Music's search filter for videos (clips). */
internal const val SEARCH_VIDEOS = "EgWKAQIQAWoMEA4QChADEAQQCRAF"

/**
 * The song and the clip of the same recording. YouTube gives only its own
 * apps the exact pair and time map, so the other half is found by search:
 * the studio song for a clip, the official clip for a song, matched by
 * title, artist and length. Without [findClip] a song costs one request
 * (what it is) and its clip isn't looked for.
 */
suspend fun Catalog.mediaPair(videoId: String, title: String, artist: String, findClip: Boolean): MediaPair {
    val data = call("next", json("videoId" to videoId, "isAudioOnly" to true), english = true)
    val entry = findAll(data, "playlistPanelVideoRenderer")
        .filterIsInstance<org.json.JSONObject>()
        .firstOrNull { it.str("videoId") == videoId }
    val kind = (entry?.let { findFirst(it, "musicVideoType") } ?: findFirst(data.opt("currentVideoEndpoint"), "musicVideoType")) as? String ?: ""
    val length = entry?.let { parseDuration(text(it.opt("lengthText"))) }?.toDouble()
    val isSong = kind == "MUSIC_VIDEO_TYPE_ATV" || kind == "MUSIC_VIDEO_TYPE_PRIVATELY_OWNED_TRACK"
    val cover = entry?.let { albumArt(it.opt("thumbnail")) }
    if (isSong && !findClip) return MediaPair(videoId, null, 0.0, cover)
    val lead = leadArtist(artist)
    val query = "$lead ${songTitle(title, lead)}"
    val rows = runCatching {
        val found = call("search", json("query" to query, "params" to if (isSong) SEARCH_VIDEOS else SearchKind.Songs.params))
        findAll(found, "musicResponsiveListItemRenderer").filterIsInstance<org.json.JSONObject>().mapNotNull { r ->
            val track = listItem(r)?.track ?: return@mapNotNull null
            track to (findFirst(r, "musicVideoType") as? String)
        }
    }.getOrDefault(emptyList())
    val other = rows.firstOrNull { (t, type) ->
        if (t.videoId == videoId || (isSong && type != "MUSIC_VIDEO_TYPE_OMV")) return@firstOrNull false
        if (!sameRecording(title, artist, t.title, t.artist)) return@firstOrNull false
        val theirs = t.durationSecs?.toDouble()
        when {
            length == null || theirs == null -> true
            // A clip runs as long as its song or longer, not much shorter.
            isSong -> theirs >= length - 3 && theirs <= length + 180
            else -> theirs <= length + 3 && theirs >= length * 0.6
        }
    }?.first ?: return MediaPair(videoId, if (isSong) null else videoId, 0.0, cover)
    val theirs = other.durationSecs?.toDouble()
    return MediaPair(
        song = if (isSong) videoId else other.videoId,
        video = if (isSong) other.videoId else videoId,
        offset = clipOffset(if (isSong) length else theirs, if (isSong) theirs else length),
        cover = cover,
    )
}

/** The largest thumbnail below [value] when it is album art (googleusercontent), not a frame of the clip (i.ytimg.com). */
internal fun albumArt(value: Any?): String? {
    val thumbs = (findFirst(value, "thumbnails") as? org.json.JSONArray)?.objects() ?: return null
    val url = thumbs.maxByOrNull { it.optInt("width") }?.str("url") ?: return null
    return url.takeIf { it.contains("googleusercontent.com") }
}

/** A longer clip is taken to have an intro of the difference (up to 45 s; beyond that more likely scenes after the song). */
internal fun clipOffset(song: Double?, video: Double?): Double =
    if (song != null && video != null && video > song && video - song <= 45) video - song else 0.0

/** Lower case, letters and digits only, words single-spaced. */
internal fun normalized(s: String): String =
    s.lowercase().map { if (it.isLetterOrDigit()) it else ' ' }.joinToString("").split(' ').filter { it.isNotEmpty() }.joinToString(" ")

/** A title without bracketed parts ("(Official Video)", "[4K]") and "feat. …", normalized. */
internal fun bareTitle(title: String): String {
    val out = StringBuilder()
    var depth = 0
    for (c in title) {
        when (c) {
            '(', '[' -> depth++
            ')', ']' -> depth = maxOf(0, depth - 1)
            else -> if (depth == 0) out.append(c)
        }
    }
    val lower = out.toString().lowercase()
    val cut = listOf(" feat.", " feat ", " ft.", " ft ").mapNotNull { n -> lower.indexOf(n).takeIf { it >= 0 } }.minOrNull() ?: lower.length
    return normalized(lower.substring(0, cut))
}

/** An artist's lead name: the first of several, without "VEVO" / "Official". */
internal fun leadArtist(artist: String): String {
    var name = normalized(artist.split(',', '&').first()).replace("vevo", "").trim()
    for (tail in listOf(" official", " music", " topic")) if (name.endsWith(tail)) name = name.removeSuffix(tail)
    return name.trim()
}

private fun related(a: String, b: String) = a.isNotEmpty() && b.isNotEmpty() && (a.contains(b) || b.contains(a))

/** A clip's title without the artist in it ("Artist - Title", "Title – Artist") or decorations; [artist] is the lead artist. */
internal fun songTitle(title: String, artist: String): String {
    val dashed = title.replace('–', '-').replace('—', '-')
    val i = dashed.indexOf(" - ")
    if (i >= 0) {
        val left = dashed.substring(0, i)
        val right = dashed.substring(i + 3)
        if (related(leadArtist(left), artist)) return bareTitle(right)
        if (related(leadArtist(bareTitle(right)), artist)) return bareTitle(left)
    }
    return bareTitle(dashed)
}

/** The same title by the same artist, decorations aside. */
internal fun sameRecording(title: String, artist: String, otherTitle: String, otherArtist: String): Boolean {
    val x = leadArtist(artist)
    val a = songTitle(title, x)
    return a.isNotEmpty() && a == songTitle(otherTitle, x) && related(x, leadArtist(otherArtist))
}

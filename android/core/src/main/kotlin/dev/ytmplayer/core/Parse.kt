package dev.ytmplayer.core

import org.json.JSONObject

// Ported from YTMKit's Parse.swift (itself from the desktop player's
// `catalog.rs`): lenient parsing, unknown renderers are skipped, never an
// error.

internal fun pageType(endpoint: Any?): String? = string(
    endpoint,
    "browseEndpoint/browseEndpointContextSupportedConfigs/browseEndpointContextMusicConfig/pageType",
)

internal fun kindOf(pageType: String?): ItemKind? = when (pageType) {
    "MUSIC_PAGE_TYPE_ALBUM", "MUSIC_PAGE_TYPE_AUDIOBOOK" -> ItemKind.Album
    // The library's own rows link to "MPLA<channel id>" pages.
    "MUSIC_PAGE_TYPE_ARTIST", "MUSIC_PAGE_TYPE_USER_CHANNEL", "MUSIC_PAGE_TYPE_LIBRARY_ARTIST" -> ItemKind.Artist
    "MUSIC_PAGE_TYPE_PLAYLIST" -> ItemKind.Playlist
    else -> null
}

/** Playlist ids travel as `PL…`; browse ids carry a `VL` prefix. */
fun bare(playlistId: String): String = playlistId.removePrefix("VL")

/** "3:52" / "1:02:03" → seconds. */
fun parseDuration(s: String): Int? {
    val parts = s.trim().split(':')
    if (parts.size < 2) return null
    var total = 0
    for (part in parts) total = total * 60 + (part.toIntOrNull() ?: return null)
    return total
}

/** The thumbnail ≥ 200 px wide (else the largest). */
internal fun thumbnail(value: Any?): String? {
    val thumbs = (findFirst(value, "thumbnails") as? org.json.JSONArray)?.objects() ?: return null
    val pick = thumbs.firstOrNull { it.optInt("width") >= 200 } ?: thumbs.lastOrNull()
    return pick?.str("url")
}

/** YouTube Music's "E" badge anywhere below `value`. */
internal fun explicitBadge(value: Any?): Boolean = findAll(value, "iconType").any { it == "MUSIC_EXPLICIT_BADGE" }

/**
 * `musicNavigationButtonRenderer`: an Explore chip ("Charts", "Chill"),
 * which opens a browse page. Mood categories need `params`, kept after a
 * `#` in the id — browse ids never contain one (`Catalog.page` splits it).
 */
internal fun navButton(r: JSONObject): Item? {
    val title = text(r.opt("buttonText"))
    val endpoint = pointer(r, "clickCommand/browseEndpoint") as? JSONObject ?: return null
    val id = endpoint.str("browseId") ?: return null
    if (title.isEmpty()) return null
    val full = endpoint.str("params")?.let { "$id#$it" } ?: id
    return Item(ItemKind.Playlist, full, title, "")
}

/**
 * `musicResponsiveListItemRenderer`: a row in search results, shelves and
 * track lists. `albumArtist` fills in album pages' artist-less rows.
 */
internal fun listItem(r: JSONObject, albumArtist: String? = null): Item? {
    val columns = r.arr("flexColumns")
    fun column(i: Int): Any? = columns?.opt(i)?.let { pointer(it, "musicResponsiveListItemFlexColumnRenderer/text") }
    val title = text(column(0))
    if (title.isEmpty()) return null
    val second = runs(column(1))
    val subtitle = text(column(1))
    val art = thumbnail(r.opt("thumbnail"))

    val videoId = string(r, "playlistItemData/videoId")
        ?: (findFirst(r, "watchEndpoint") as? JSONObject)?.str("videoId")
    val endpoint = r.obj("navigationEndpoint")
    if (endpoint?.has("browseEndpoint") == true) {
        val kind = kindOf(pageType(endpoint)) ?: return null
        val id = string(endpoint, "browseEndpoint/browseId") ?: return null
        return Item(kind, id, title, subtitle, art, explicit = explicitBadge(r))
    }
    videoId ?: return null
    // Artists are the runs linking to artist pages; else the first run.
    val artists = second.filter { kindOf(pageType(it.opt("navigationEndpoint"))) == ItemKind.Artist }
        .mapNotNull { it.str("text") }
    val artist = if (artists.isNotEmpty()) artists.joinToString(", ")
    else albumArtist ?: second.firstOrNull()?.str("text") ?: ""
    val fixed = r.arr("fixedColumns")?.opt(0)
    val duration = parseDuration(text(pointer(fixed, "musicResponsiveListItemFixedColumnRenderer/text")))
        ?: second.reversed().firstNotNullOfOrNull { it.str("text")?.let(::parseDuration) }
    val explicit = explicitBadge(r)
    val track = Track(videoId, title, cleanArtist(artist), duration, explicit)
    return Item(ItemKind.Song, videoId, title, subtitle, art, track, explicit)
}

/** `musicTwoRowItemRenderer`: a card (album, playlist, artist, song). */
internal fun card(r: JSONObject): Item? {
    val title = text(r.opt("title"))
    val subtitle = text(r.opt("subtitle"))
    val endpoint = r.obj("navigationEndpoint") ?: return null
    val art = thumbnail(r.opt("thumbnailRenderer"))
    val explicit = explicitBadge(r)
    string(endpoint, "watchEndpoint/videoId")?.let { videoId ->
        val artist = runs(r.opt("subtitle")).firstOrNull { it.has("navigationEndpoint") }?.str("text") ?: ""
        val track = Track(videoId, title, cleanArtist(artist), explicit = explicit)
        return Item(ItemKind.Song, videoId, title, subtitle, art, track, explicit)
    }
    val kind = kindOf(pageType(endpoint)) ?: return null
    val id = string(endpoint, "browseEndpoint/browseId") ?: return null
    return Item(kind, id, title, subtitle, art, explicit = explicit)
}

fun parseSearch(data: Any, kind: SearchKind): List<Item> {
    val want = when (kind) {
        SearchKind.Songs -> ItemKind.Song
        SearchKind.Albums -> ItemKind.Album
        SearchKind.Artists -> ItemKind.Artist
        SearchKind.Playlists -> ItemKind.Playlist
    }
    return findAll(data, "musicResponsiveListItemRenderer")
        .mapNotNull { (it as? JSONObject)?.let { r -> listItem(r) } }
        .filter { it.kind == want }
}

/**
 * Playlist rows as `(song, setVideoId)`; the set id is what removing that
 * entry needs, and is empty when YouTube doesn't give one.
 */
fun playlistEntriesIn(data: Any): List<Pair<Track, String>> =
    findAll(data, "musicResponsiveListItemRenderer").mapNotNull { row ->
        val r = row as? JSONObject ?: return@mapNotNull null
        val track = listItem(r)?.track ?: return@mapNotNull null
        track to (string(r, "playlistItemData/playlistSetVideoId") ?: "")
    }

fun songsIn(data: Any, albumArtist: String? = null): List<Track> =
    findAll(data, "musicResponsiveListItemRenderer")
        .mapNotNull { (it as? JSONObject)?.let { r -> listItem(r, albumArtist)?.track } }

/**
 * Carousels of cards (and of song rows) with their titles, then the grids
 * Explore and "moods & genres" use instead.
 */
fun parseShelves(data: Any): List<Shelf> {
    val carousels = findAll(data, "musicCarouselShelfRenderer").mapNotNull { shelf ->
        val title = text(pointer(shelf, "header/musicCarouselShelfBasicHeaderRenderer/title"))
        val items = shelfItems((shelf as? JSONObject)?.arr("contents")?.objects().orEmpty())
        if (items.isEmpty()) null else Shelf(title, items)
    }
    val grids = findAll(data, "gridRenderer").mapNotNull { grid ->
        val title = text(pointer(grid, "header/gridHeaderRenderer/title"))
        val items = shelfItems((grid as? JSONObject)?.arr("items")?.objects().orEmpty())
        if (items.isEmpty()) null else Shelf(title, items)
    }
    return carousels + grids
}

/** The cards of a carousel or grid, whichever renderer they use. */
internal fun shelfItems(contents: List<JSONObject>): List<Item> = contents.mapNotNull { c ->
    c.obj("musicTwoRowItemRenderer")?.let(::card)
        ?: c.obj("musicResponsiveListItemRenderer")?.let { listItem(it) }
        ?: c.obj("musicNavigationButtonRenderer")?.let(::navButton)
}

/** The playlist behind an artist page's "Top songs" shelf. */
fun topSongsPlaylist(data: Any): String? {
    for (shelf in findAll(data, "musicShelfRenderer")) {
        val endpoint = (shelf as? JSONObject)?.opt("bottomEndpoint")?.orNull()
            ?: pointer(shelf, "title/runs/0/navigationEndpoint")
        val id = string(endpoint, "browseEndpoint/browseId")
        if (id != null && id.startsWith("VL")) return id
    }
    return null
}

fun parsePage(id: String, data: Any): Page {
    val kind = when {
        id.startsWith("MPRE") -> ItemKind.Album
        id.startsWith("UC") -> ItemKind.Artist
        else -> ItemKind.Playlist
    }
    val header = listOf(
        "musicImmersiveHeaderRenderer", "musicVisualHeaderRenderer",
        "musicResponsiveHeaderRenderer", "musicDetailHeaderRenderer",
    ).firstNotNullOfOrNull { findFirst(data, it) as? JSONObject }
    val title = text(header?.opt("title"))
    val strapline = text(header?.opt("straplineTextOne"))
    val sub = text(header?.opt("subtitle"))
    val subtitle = when {
        kind == ItemKind.Album && strapline.isNotEmpty() -> "$sub • $strapline"
        sub.isNotEmpty() -> sub
        else -> text(header?.opt("monthlyListenerCount"))
    }
    val albumArtist = if (kind == ItemKind.Album && strapline.isNotEmpty()) strapline else null
    val tracks = listOf("musicShelfRenderer", "musicPlaylistShelfRenderer")
        .flatMap { findAll(data, it) }
        .flatMap { songsIn(it, albumArtist) }
    // Video carousels duplicate the songs; keep albums, artists, lists.
    val shelves = parseShelves(data).filter { s -> s.items.any { it.kind != ItemKind.Song } }
    return Page(kind, id, title, subtitle, thumbnail(header?.opt("thumbnail")), tracks, shelves)
}

fun parseRadio(data: Any): List<Track> = findAll(data, "playlistPanelVideoRenderer").mapNotNull { value ->
    val r = value as? JSONObject ?: return@mapNotNull null
    val videoId = r.str("videoId") ?: return@mapNotNull null
    val artist = runs(r.opt("longBylineText")).firstOrNull()?.str("text") ?: ""
    Track(videoId, text(r.opt("title")), cleanArtist(artist), parseDuration(text(r.opt("lengthText"))))
}

fun tabBrowseId(next: Any, title: String): String? {
    val tab = findAll(next, "tabRenderer").firstOrNull { (it as? JSONObject)?.str("title") == title }
    return string(tab, "endpoint/browseEndpoint/browseId")
}

/** Plain query strings from `music/get_search_suggestions`. */
fun parseSuggestions(data: Any): List<String> = findAll(data, "searchSuggestionRenderer")
    .map { text((it as? JSONObject)?.opt("suggestion")) }
    .filter { it.isNotEmpty() }

fun parseLyricsText(data: Any): Pair<String, String>? {
    val shelf = findFirst(data, "musicDescriptionShelfRenderer") as? JSONObject ?: return null
    val body = text(shelf.opt("description"))
    return if (body.isEmpty()) null else body to text(shelf.opt("footer"))
}

fun continuation(data: Any): String? {
    (findFirst(data, "nextContinuationData") as? JSONObject)?.str("continuation")?.let { return it }
    return (findFirst(data, "continuationCommand") as? JSONObject)?.str("token")
}

/** The song's first artist page, as linked under the song in the queue. */
fun artistIdIn(next: Any, videoId: String): String? {
    val items = findAll(next, "playlistPanelVideoRenderer").mapNotNull { it as? JSONObject }
    val song = items.firstOrNull { it.str("videoId") == videoId } ?: items.firstOrNull()
    return runs(song?.opt("longBylineText"))
        .mapNotNull { string(it, "navigationEndpoint/browseEndpoint/browseId") }
        .firstOrNull { it.startsWith("UC") }
}

/** Cards in a library grid (playlists, albums), not in carousels. */
fun libraryCards(data: Any): List<Item> = findAll(data, "musicTwoRowItemRenderer")
    .mapNotNull { (it as? JSONObject)?.let(::card) }
    .filter { it.kind == ItemKind.Playlist || it.kind == ItemKind.Album }

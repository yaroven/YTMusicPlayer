package dev.yarek.core

import org.json.JSONObject

/**
 * YouTube Music's own API (InnerTube, WEB_REMIX client): quota-free, no
 * sign-in needed. With `auth` signed in, requests go as the account
 * (personal home, its library). Mirrors YTMKit's Catalog.swift.
 */
class Catalog(private val auth: Auth? = null) {
    /** The language shelves, titles and chips come in ("en", "uk"). */
    @Volatile
    var language: String = "en"

    /** Whether requests go as an account. */
    val signedIn: Boolean get() = auth?.signedIn == true

    /**
     * `english`: the answer is parsed by its English labels (the song's
     * "Lyrics" / "Related" tabs). `visitor`: continuations of a feed need
     * the visitor id its first answer came with.
     */
    internal suspend fun call(endpoint: String, body: JSONObject, english: Boolean = false, visitor: String? = null): JSONObject {
        val hl = if (english) "en" else language
        body.put(
            "context",
            json(
                "client" to mapOf(
                    "clientName" to "WEB_REMIX", "clientVersion" to CLIENT_VERSION,
                    "hl" to hl, "gl" to if (hl == "uk") "UA" else "US", "visitorData" to visitor,
                ),
            ),
        )
        val headers = mutableMapOf(
            "Content-Type" to "application/json",
            "Origin" to "https://music.youtube.com",
            "User-Agent" to USER_AGENT,
        )
        val signed = auth?.takeIf { it.signedIn }
        signed?.let { headers += it.headers("https://music.youtube.com") }
        val response = Http.send("$BASE/$endpoint?prettyPrint=false", "POST", headers, body.toString().toByteArray())
        signed?.absorb(response)
        if (!response.ok) throw YTMError.Http("YouTube Music $endpoint", response.status)
        return JSONObject(response.text)
    }

    suspend fun search(query: String, kind: SearchKind): List<Item> =
        parseSearch(call("search", json("query" to query, "params" to kind.params)), kind)

    /**
     * Album (`MPREb_…`), artist (`UC…`) or playlist (`VL…` / `PL…`) page.
     * With `full = false` only the first answer: up to 100 songs, the rest
     * left in `continuation` (an artist's 5 top songs, the rest in
     * `allSongsId`), so a screen can show it at once and add the rest.
     */
    suspend fun page(browseId: String, full: Boolean = true): Page {
        // Explore chips carry their params after a `#` (see `navButton`).
        val raw = browseId.substringBefore('#')
        val params = browseId.substringAfter('#', "").ifEmpty { null }
        val id = if (listOf("PL", "OLAK", "RD").any { raw.startsWith(it) }) "VL$raw" else raw
        val data = call("browse", json("browseId" to id, "params" to params))
        var page = parsePage(id, data)
        if (!full) return page.copy(continuation = if (page.kind == ItemKind.Playlist) continuation(data) else null)
        // Long playlists come in pages.
        var token = if (page.kind == ItemKind.Playlist) continuation(data) else null
        var pages = 1
        while (token != null && pages < MAX_PAGES) {
            pages++
            val more = call("browse", json("continuation" to token))
            val extra = songsIn(more)
            if (extra.isEmpty()) break
            page = page.copy(tracks = page.tracks + extra)
            token = continuation(more)
        }
        // An artist page lists 5 top songs; its "Show all" playlist has them all.
        if (page.kind == ItemKind.Artist) {
            val all = topSongsPlaylist(data)
            if (all != null) {
                val list = runCatching { page(all) }.getOrNull()
                if (list != null && list.tracks.size > page.tracks.size) page = page.copy(tracks = list.tracks)
            }
        }
        return page
    }

    /** The next part of a long track list, and the token for the one after. */
    suspend fun moreTracks(token: String): Pair<List<Track>, String?> {
        val more = call("browse", json("continuation" to token))
        val tracks = songsIn(more)
        return tracks to if (tracks.isEmpty()) null else continuation(more)
    }

    /** Explore: charts, new releases, moods and genres. */
    suspend fun explore(): List<Shelf> = parseShelves(call("browse", json("browseId" to "FEmusic_explore")))

    /** What YouTube Music suggests while you type. */
    suspend fun suggestions(query: String): List<String> {
        val q = query.trim()
        if (q.isEmpty()) return emptyList()
        return parseSuggestions(call("music/get_search_suggestions", json("input" to q)))
    }

    suspend fun home(): List<Shelf> = parseShelves(call("browse", json("browseId" to "FEmusic_home")))

    /** Home's first shelves and mood chips; with `params` (a chip's), Home for that mood. */
    suspend fun homeFeed(params: String? = null): HomeFeed {
        val data = call("browse", json("browseId" to "FEmusic_home", "params" to params))
        return HomeFeed(parseHomeChips(data), parseShelves(data), homeContinuation(data), string(data, "responseContext/visitorData"))
    }

    /** Home's next shelves (three or so at a time) and the token after. */
    suspend fun moreHome(token: String, visitor: String?): Pair<List<Shelf>, String?> {
        val data = call("browse", json("continuation" to token), visitor = visitor)
        val shelves = parseShelves(data)
        return shelves to if (shelves.isEmpty()) null else homeContinuation(data)
    }

    /** What YouTube Music plays after the song (≈50 songs, the song first). */
    suspend fun radio(videoId: String): List<Track> = parseRadio(next(videoId))

    suspend fun related(videoId: String): List<Shelf> {
        val id = tabBrowseId(next(videoId), "Related") ?: return emptyList()
        return parseShelves(call("browse", json("browseId" to id)))
    }

    /** Plain lyrics from YouTube Music's provider. */
    suspend fun lyrics(videoId: String): Lyrics? {
        val id = tabBrowseId(next(videoId), "Lyrics") ?: return null
        val (body, source) = parseLyricsText(call("browse", json("browseId" to id))) ?: return null
        return Lyrics(body.split('\n').map { LyricLine(null, it) }, source)
    }

    /** The song's (first) artist page id. */
    suspend fun artistId(videoId: String): String? = artistIdIn(next(videoId), videoId)

    /** The signed-in account's name, to show who is signed in. */
    suspend fun accountName(): String? = accountInfo()?.name

    /** The signed-in account's name and picture. */
    suspend fun accountInfo(): AccountInfo? {
        val data = call("account/account_menu", JSONObject())
        val name = text(findFirst(data, "accountName")).ifEmpty { return null }
        return AccountInfo(name, thumbnail(findFirst(data, "accountPhoto")))
    }

    suspend fun likedSongs(): List<Track> = page("VLLM").tracks

    /** The account's own playlists (not Liked Music, not "New playlist"). */
    suspend fun libraryPlaylists(): List<Item> =
        libraryCards(call("browse", json("browseId" to "FEmusic_liked_playlists")))
            .filter { it.kind == ItemKind.Playlist && it.browseId != "VLLM" && it.browseId != "LM" }

    suspend fun libraryAlbums(): List<Item> =
        libraryCards(call("browse", json("browseId" to "FEmusic_liked_albums"))).filter { it.kind == ItemKind.Album }

    /** Artists the account has songs or albums from; ids are channel ids. */
    suspend fun libraryArtists(): List<Item> =
        findAll(call("browse", json("browseId" to "FEmusic_library_corpus_track_artists")), "musicResponsiveListItemRenderer")
            .mapNotNull { (it as? JSONObject)?.let { r -> listItem(r) } }
            .filter { it.kind == ItemKind.Artist }
            .map { if (it.browseId.startsWith("MPLA")) it.copy(browseId = it.browseId.removePrefix("MPLA")) else it }

    /** A playlist's songs with each entry's `setVideoId`. Paged like `page`. */
    suspend fun playlistEntries(playlistId: String): List<Pair<Track, String>> {
        val data = call("browse", json("browseId" to "VL" + bare(playlistId)))
        val entries = playlistEntriesIn(data).toMutableList()
        var token = continuation(data)
        var pages = 1
        while (token != null && pages < MAX_PAGES) {
            pages++
            val more = call("browse", json("continuation" to token))
            val extra = playlistEntriesIn(more)
            if (extra.isEmpty()) break
            entries += extra
            token = continuation(more)
        }
        return entries
    }

    // Writes (signed in): the same endpoints the YouTube Music site uses.

    suspend fun rate(videoId: String, rating: Rating) {
        val endpoint = when (rating) {
            Rating.Like -> "like/like"
            Rating.Dislike -> "like/dislike"
            Rating.None -> "like/removelike"
        }
        write(endpoint, json("target" to mapOf("videoId" to videoId)))
    }

    suspend fun subscribe(channelId: String, on: Boolean) {
        write(if (on) "subscription/subscribe" else "subscription/unsubscribe", json("channelIds" to listOf(channelId)))
    }

    /** Creates a private playlist; returns its id (`PL…`). */
    suspend fun createPlaylist(title: String, videoIds: List<String> = emptyList()): String {
        val body = json("title" to title, "privacyStatus" to "PRIVATE")
        if (videoIds.isNotEmpty()) body.put("videoIds", toJson(videoIds))
        val data = write("playlist/create", body)
        val id = data.str("playlistId") ?: findFirst(data, "playlistId") as? String
            ?: throw YTMError.Http("YouTube Music didn't return the new playlist's id", 200)
        return bare(id)
    }

    suspend fun deletePlaylist(playlistId: String) {
        write("playlist/delete", json("playlistId" to bare(playlistId)))
    }

    suspend fun renamePlaylist(playlistId: String, title: String) {
        editPlaylist(playlistId, mapOf("action" to "ACTION_SET_PLAYLIST_NAME", "playlistName" to title))
    }

    /** Adds a song to a playlist; returns its `setVideoId`. */
    suspend fun addToPlaylist(playlistId: String, videoId: String): String =
        findFirst(editPlaylist(playlistId, mapOf("action" to "ACTION_ADD_VIDEO", "addedVideoId" to videoId)), "setVideoId")
            as? String ?: ""

    /** Moves `setVideoId` in front of `before`, or to the end when null. */
    suspend fun moveInPlaylist(playlistId: String, setVideoId: String, before: String?) {
        editPlaylist(
            playlistId,
            mapOf("action" to "ACTION_MOVE_VIDEO_BEFORE", "setVideoId" to setVideoId, "movedSetVideoIdSuccessor" to before),
        )
    }

    suspend fun removeFromPlaylist(playlistId: String, videoId: String, setVideoId: String) {
        editPlaylist(
            playlistId,
            mapOf("action" to "ACTION_REMOVE_VIDEO", "removedVideoId" to videoId, "setVideoId" to setVideoId),
        )
    }

    private suspend fun editPlaylist(playlistId: String, action: Map<String, Any?>): JSONObject =
        write("browse/edit_playlist", json("playlistId" to bare(playlistId), "actions" to listOf(action)))

    /** A write needs the sign-in; YouTube reports failures in the body. */
    private suspend fun write(endpoint: String, body: JSONObject): JSONObject {
        if (!signedIn) throw YTMError.SignInRequired("Sign in with Google to change your library")
        val data = call(endpoint, body)
        val status = data.str("status")
        if (status != null && status != "STATUS_SUCCEEDED") throw YTMError.Http("YouTube Music $endpoint: $status", 200)
        return data
    }

    private suspend fun next(videoId: String): JSONObject =
        call("next", json("videoId" to videoId, "playlistId" to "RDAMVM$videoId", "isAudioOnly" to true), english = true)

    companion object {
        const val BASE = "https://music.youtube.com/youtubei/v1"
        const val CLIENT_VERSION = "1.20260901.01.00"
        const val MAX_PAGES = 10
        const val USER_AGENT =
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Safari/605.1.15"
    }
}

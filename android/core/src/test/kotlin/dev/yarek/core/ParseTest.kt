package dev.yarek.core

import org.json.JSONObject
import java.io.File
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith
import kotlin.test.assertNull
import kotlin.test.assertTrue

// The offline half of YTMKitChecks, same fixtures.

private fun browse(id: String, type: String) =
    """{"browseEndpoint":{"browseId":"$id","browseEndpointContextSupportedConfigs":{"browseEndpointContextMusicConfig":{"pageType":"$type"}}}}"""

private fun column(runs: String) = """{"musicResponsiveListItemFlexColumnRenderer":{"text":{"runs":$runs}}}"""

private val songRow = """
{"musicResponsiveListItemRenderer":{
  "flexColumns":[${column("""[{"text":"Dancing Queen"}]""")},
    ${column("""[{"text":"ABBA","navigationEndpoint":${browse("UCabba", "MUSIC_PAGE_TYPE_ARTIST")}},{"text":" • "},{"text":"Arrival","navigationEndpoint":${browse("MPREb_x", "MUSIC_PAGE_TYPE_ALBUM")}},{"text":" • "},{"text":"3:52"}]""")}],
  "playlistItemData":{"videoId":"YkLLcIKhJ64"},
  "thumbnail":{"musicThumbnailRenderer":{"thumbnail":{"thumbnails":[{"url":"https://a/small","width":60},{"url":"https://a/big","width":226}]}}}}}
"""

private val albumRow = """
{"musicResponsiveListItemRenderer":{
  "flexColumns":[${column("""[{"text":"ABBA Gold"}]""")},
    ${column("""[{"text":"Album"},{"text":" • "},{"text":"ABBA"},{"text":" • "},{"text":"1992"}]""")}],
  "navigationEndpoint":${browse("MPREb_gold", "MUSIC_PAGE_TYPE_ALBUM")}}}
"""

private fun j(s: String) = JSONObject(s)

class ParseTest {
    @Test
    fun searchRows() {
        val search = j("""{"contents":[$songRow,$albumRow]}""")
        val songs = parseSearch(search, SearchKind.Songs)
        assertEquals(1, songs.size)
        val t = songs[0].track!!
        assertEquals(Triple("Dancing Queen", "ABBA", 232), Triple(t.title, t.artist, t.durationSecs))
        assertEquals("https://a/big", songs[0].thumbnail)
        val albums = parseSearch(search, SearchKind.Albums)
        assertEquals(listOf("MPREb_gold"), albums.map { it.browseId })
        assertEquals("Album • ABBA • 1992", albums[0].subtitle)
    }

    @Test
    fun albumPage() {
        val album = j(
            """
            {"header":{"musicResponsiveHeaderRenderer":{"title":{"runs":[{"text":"ABBA Gold"}]},
               "subtitle":{"runs":[{"text":"Album"},{"text":" • "},{"text":"1992"}]},
               "straplineTextOne":{"runs":[{"text":"ABBA"}]}}},
             "contents":{"musicShelfRenderer":{"contents":[{"musicResponsiveListItemRenderer":{
               "flexColumns":[${column("""[{"text":"Mamma Mia"}]""")},${column("[]")}],
               "fixedColumns":[{"musicResponsiveListItemFixedColumnRenderer":{"text":{"runs":[{"text":"3:33"}]}}}],
               "playlistItemData":{"videoId":"unFR0ZXmB0E"}}}]}}}
            """,
        )
        val page = parsePage("MPREb_gold", album)
        assertEquals(ItemKind.Album, page.kind)
        assertEquals("ABBA Gold", page.title)
        assertEquals("Album • 1992 • ABBA", page.subtitle)
        assertEquals(listOf("ABBA" to 213), page.tracks.map { it.artist to it.durationSecs })
    }

    @Test
    fun artistPage() {
        val artist = j(
            """
            {"header":{"musicImmersiveHeaderRenderer":{"title":{"runs":[{"text":"ABBA"}]}}},
             "contents":[{"musicShelfRenderer":{"contents":[$songRow],
                 "bottomEndpoint":{"browseEndpoint":{"browseId":"VLOLAK5uy_top"}}}},
               {"musicCarouselShelfRenderer":{"header":{"musicCarouselShelfBasicHeaderRenderer":{"title":{"runs":[{"text":"Albums"}]}}},
                 "contents":[{"musicTwoRowItemRenderer":{"title":{"runs":[{"text":"Arrival"}]},"subtitle":{"runs":[{"text":"1976"}]},
                   "navigationEndpoint":${browse("MPREb_arrival", "MUSIC_PAGE_TYPE_ALBUM")}}}]}},
               {"musicCarouselShelfRenderer":{"header":{"musicCarouselShelfBasicHeaderRenderer":{"title":{"runs":[{"text":"Videos"}]}}},
                 "contents":[{"musicTwoRowItemRenderer":{"title":{"runs":[{"text":"Dancing Queen"}]},
                   "navigationEndpoint":{"watchEndpoint":{"videoId":"x"}}}}]}}]}
            """,
        )
        val page = parsePage("UCabba", artist)
        assertEquals(ItemKind.Artist, page.kind)
        assertEquals(1, page.tracks.size)
        assertEquals(listOf("Albums"), page.shelves.map { it.title })
        assertEquals(ItemKind.Album, page.shelves[0].items[0].kind)
        assertEquals("VLOLAK5uy_top", topSongsPlaylist(artist))
    }

    @Test
    fun radioTabsArtist() {
        val next = j(
            """
            {"contents":[{"playlistPanelVideoRenderer":{"videoId":"5mHzaIehRTE",
                "title":{"runs":[{"text":"Lay All Your Love On Me"}]},
                "longBylineText":{"runs":[{"text":"ABBA","navigationEndpoint":{"browseEndpoint":{"browseId":"UCabba"}}},{"text":" • "},{"text":"Super Trouper"}]},
                "lengthText":{"runs":[{"text":"4:35"}]}}},
              {"tabRenderer":{"title":"Lyrics","endpoint":{"browseEndpoint":{"browseId":"MPLYt_x"}}}}]}
            """,
        )
        val radio = parseRadio(next)
        assertEquals(listOf("ABBA" to 275), radio.map { it.artist to it.durationSecs })
        assertEquals("MPLYt_x", tabBrowseId(next, "Lyrics"))
        assertNull(tabBrowseId(next, "Related"))
        assertEquals("UCabba", artistIdIn(next, "5mHzaIehRTE"))
        val lyrics = parseLyricsText(
            j("""{"musicDescriptionShelfRenderer":{"description":{"runs":[{"text":"Half-past twelve\nAnd I'm watching"}]},"footer":{"runs":[{"text":"Source: Musixmatch"}]}}}"""),
        )
        assertTrue(lyrics!!.first.startsWith("Half-past"))
        assertEquals("Source: Musixmatch", lyrics.second)
    }

    @Test
    fun library() {
        val likedRow = songRow.replace(
            """"playlistItemData":{"videoId":"YkLLcIKhJ64"}""",
            """"playlistItemData":{"videoId":"YkLLcIKhJ64","playlistSetVideoId":"set-1"}""",
        )
        val entries = playlistEntriesIn(j("""{"contents":[$likedRow,$songRow]}"""))
        assertEquals(listOf("set-1", ""), entries.map { it.second })
        assertEquals("PLroad", bare("VLPLroad"))
        assertEquals("PLroad", bare("PLroad"))
        fun libraryCard(title: String, id: String, type: String) =
            """{"musicTwoRowItemRenderer":{"title":{"runs":[{"text":"$title"}]},"subtitle":{"runs":[{"text":"Playlist"}]},"navigationEndpoint":${browse(id, type)}}}"""
        val lists = j(
            """{"contents":[{"musicTwoRowItemRenderer":{"title":{"runs":[{"text":"New playlist"}]}}},
              ${libraryCard("Liked Music", "VLLM", "MUSIC_PAGE_TYPE_PLAYLIST")},
              ${libraryCard("Road trip", "VLPLroad", "MUSIC_PAGE_TYPE_PLAYLIST")},
              ${libraryCard("ABBA Gold", "MPREb_gold", "MUSIC_PAGE_TYPE_ALBUM")}]}""",
        )
        val own = libraryCards(lists).filter { it.kind == ItemKind.Playlist && it.browseId != "VLLM" }
        assertEquals(listOf("VLPLroad"), own.map { it.browseId })
        val artistRow = j(
            """{"contents":[{"musicResponsiveListItemRenderer":{
              "flexColumns":[${column("""[{"text":"Korn"}]""")},${column("""[{"text":"Artist"}]""")}],
              "navigationEndpoint":${browse("MPLAUCkorn", "MUSIC_PAGE_TYPE_LIBRARY_ARTIST")}}}]}""",
        )
        val row = findAll(artistRow, "musicResponsiveListItemRenderer").firstNotNullOf { listItem(it as JSONObject) }
        assertEquals(ItemKind.Artist, row.kind)
    }

    @Test
    fun durationsAndTitles() {
        assertEquals(3723, parseDuration("1:02:03"))
        assertNull(parseDuration("1.7B plays"))
        assertNull(parseDuration("42"))
        assertEquals("ABBA" to "Dancing Queen", cleanTitle("ABBA", "ABBA - Dancing Queen (Official Music Video Remastered)"))
        assertEquals("Rick Astley" to "Never Gonna Give You Up", cleanTitle("RickAstleyVEVO", "Rick Astley - Never Gonna Give You Up [4K]"))
        assertEquals("Daft Punk" to "Get Lucky", cleanTitle("Daft Punk", "Get Lucky feat. Pharrell Williams"))
        assertEquals("Queen" to "Bohemian Rhapsody", cleanTitle("Queen", "Bohemian Rhapsody"))
    }

    @Test
    fun karaoke() {
        val k = Lyrics(parseLrc("[00:10.00] One two three\n[00:20.00]\n[00:30.00] Last"), "LRCLIB")
        // Only lines timed: no guessed word times, the line lights up whole.
        assertTrue(k.wordTimes(0).isEmpty())
        assertEquals(LyricsProgress(0, 3, 13), k.progress(10.1))
        // Kept on disk and read back, word times too.
        assertEquals(k, Lyrics.fromJson(k.toJson()))
        assertTrue(k.lines[1].instrumental)
        assertEquals(LyricsProgress(1, 3), k.progress(28.0))
        assertTrue(k.hasIntro)
        assertEquals(LyricsProgress(-1, 2), k.progress(5.0))
        val exact = Lyrics(parseLrc("[00:05.00] <00:05.00>Hello <00:07.00>world"), "LRCLIB")
        assertEquals("Hello world", exact.lines[0].text)
        assertEquals(listOf("Hello ", "world"), exact.lines[0].words)
        assertEquals(listOf(5.0, 7.0), exact.wordTimes(0))
        assertEquals(LyricsProgress(0, 1, 6), exact.progress(6.0))
        assertEquals(exact, Lyrics.fromJson(exact.toJson()))
        val lrc = Lyrics(parseLrc("[ar:ABBA]\n[00:17.37] Ooh, you can dance\n[00:25.18] Having the time\n[01:02.00]"), "LRCLIB")
        assertEquals(3, lrc.lines.size)
        assertTrue(lrc.synced)
        assertEquals(listOf(null, 0, 1), listOf(10.0, 20.0, 30.0).map { lrc.currentLine(it) })
    }

    @Test
    fun playerResponses() {
        val ok = j(
            """{"playabilityStatus":{"status":"OK"},"streamingData":{"expiresInSeconds":"21540","adaptiveFormats":[
              {"itag":251,"url":"https://v/opus","mimeType":"audio/webm"},
              {"itag":140,"url":"https://v/aac","mimeType":"audio/mp4","contentLength":"3500000","approxDurationMs":"230399","loudnessDb":3.68}]}}""",
        )
        val now = 1_000_000L
        val stream = parsePlayer(ok, nowMillis = now)
        assertEquals("https://v/aac", stream.url)
        assertEquals(3_500_000L, stream.contentLength)
        assertEquals(3.68, stream.loudnessDb)
        assertEquals(now + 21_540_000L, stream.expiresAtMillis)
        assertEquals(230.399, stream.duration)
        val e = assertFailsWith<YTMError.SignInRequired> {
            parsePlayer(j("""{"playabilityStatus":{"status":"LOGIN_REQUIRED","reason":"Sign in to confirm your age"}}"""))
        }
        assertEquals("Sign in to confirm your age", e.message)
        assertEquals(4.0, trackLoudness(j("""{"playerConfig":{"audioConfig":{"trackAbsoluteLoudnessLkfs":-10.0,"loudnessTargetLkfs":-14.0}}}""")))

        val hlsOnly = j(
            """{"playabilityStatus":{"status":"OK"},"videoDetails":{"lengthSeconds":"201"},
             "streamingData":{"hlsManifestUrl":"https://m/master.m3u8","adaptiveFormats":[{"itag":140,"signatureCipher":"s=x"}]}}""",
        )
        assertFailsWith<YTMError.NoStream> { parsePlayer(hlsOnly) }
        val viaHls = parsePlayer(hlsOnly, hls = true)
        assertTrue(viaHls.hls)
        assertEquals(201.0, viaHls.duration)
        assertEquals("https://m/master.m3u8", viaHls.url)
        val master = """
            #EXTM3U
            #EXT-X-MEDIA:URI="https://m/api/manifest/hls_playlist/itag/234/index.m3u8",TYPE=AUDIO,GROUP-ID="234",NAME="Default"
            #EXT-X-MEDIA:URI="https://m/api/manifest/hls_playlist/itag/140/index.m3u8",TYPE=AUDIO,GROUP-ID="140",NAME="Default"
            #EXT-X-STREAM-INF:BANDWIDTH=1,CODECS="avc1",AUDIO="234"
            https://m/video.m3u8
        """.trimIndent()
        assertEquals("https://m/api/manifest/hls_playlist/itag/140/index.m3u8", audioPlaylist(master, "https://m/master.m3u8"))
        val variants = """
            #EXTM3U
            #EXT-X-STREAM-INF:BANDWIDTH=900,CODECS="avc1.4d401e,mp4a.40.2"
            hi/index.m3u8
            #EXT-X-STREAM-INF:BANDWIDTH=300,CODECS="avc1.4d400c,mp4a.40.2"
            lo/index.m3u8
        """.trimIndent()
        assertEquals("https://m/a/lo/index.m3u8", aacVariant(variants, "https://m/a/master.m3u8"))

        val muxed = j(
            """{"playabilityStatus":{"status":"OK"},"streamingData":{"formats":[{"itag":18,"url":"https://v/18","mimeType":"video/mp4"}],
             "adaptiveFormats":[{"itag":140,"signatureCipher":"s=x&url=https://v/140"}]}}""",
        )
        val fallback = parsePlayer(muxed, itags = listOf(140, 18))
        assertEquals("https://v/18", fallback.url)
        assertTrue(fallback.hasVideo)
        assertEquals("Cgt abc%3D%3D", visitorDataIn("""x "VISITOR_DATA":"Cgt abc%3D%3D","y""""))
        assertEquals("https://v/a?b=1&n=x%2By", withParam("https://v/a?n=old&b=1", "n", "x+y"))
    }

    @Test
    fun homeChipsAndShelfStyles() {
        val home = j(
            """{"contents":[{"chipCloudChipRenderer":{"text":{"runs":[{"text":"Relax"}]},
                "navigationEndpoint":{"browseEndpoint":{"browseId":"FEmusic_home","params":"relax"}}}},
              {"musicCarouselShelfRenderer":{"header":{"musicCarouselShelfBasicHeaderRenderer":{
                  "title":{"runs":[{"text":"Quick picks"}]},"strapline":{"runs":[{"text":"JANE"}]}}},
                "contents":[$songRow]}},
              {"musicCarouselShelfRenderer":{"header":{"musicCarouselShelfBasicHeaderRenderer":{"title":{"runs":[{"text":"Videos"}]}}},
                "contents":[{"musicTwoRowItemRenderer":{"title":{"runs":[{"text":"Clip"}]},"aspectRatio":"MUSIC_TWO_ROW_ITEM_THUMBNAIL_ASPECT_RATIO_RECTANGLE_16_9",
                  "navigationEndpoint":{"watchEndpoint":{"videoId":"x"}}}}]}}],
             "continuations":[{"nextContinuationData":{"continuation":"more"}}]}""",
        )
        assertEquals(listOf(HomeChip("Relax", "relax")), parseHomeChips(home))
        val shelves = parseShelves(home)
        assertEquals(listOf(ShelfStyle.Grid, ShelfStyle.Wide), shelves.map { it.style })
        assertEquals("JANE", shelves[0].strapline)
        assertEquals("more", homeContinuation(home))
    }

    @Test
    fun followsTheTrackListsContinuation() {
        // A playlist page has the section list's continuation (more shelves)
        // as well as its track list's: the track list's is the one to follow.
        val page = j(
            """{"contents":{"sectionListRenderer":{"continuations":[{"nextContinuationData":{"continuation":"shelves"}}],
              "contents":[{"musicPlaylistShelfRenderer":{"contents":[$songRow,
                {"continuationItemRenderer":{"continuationEndpoint":{"continuationCommand":{"token":"songs"}}}}]}}]}}}""",
        )
        assertEquals("songs", continuation(page))
    }

    @Test
    fun clientsJsonMatchesBuiltIn() {
        // ios/clients.json is what both phone apps fetch; it must decode to the built-in list.
        val file = generateSequence(File("").absoluteFile) { it.parentFile }
            .map { File(it, "ios/clients.json") }.first { it.exists() }
        assertEquals(ClientProfile.builtIn, ClientProfile.listFromJson(file.readText()))
    }

    @Test
    fun badgesChipsSuggestions() {
        val badged = j("""{"contents":[$songRow]}""")
        badged.getJSONArray("contents").getJSONObject(0).getJSONObject("musicResponsiveListItemRenderer")
            .put("badges", j("""{"a":[{"musicInlineBadgeRenderer":{"icon":{"iconType":"MUSIC_EXPLICIT_BADGE"}}}]}""").getJSONArray("a"))
        val marked = parseSearch(badged, SearchKind.Songs)
        assertTrue(marked[0].explicit && marked[0].track!!.explicit)
        assertTrue(!parseSearch(j("""{"contents":[$songRow]}"""), SearchKind.Songs)[0].explicit)

        val chips = j(
            """{"contents":{"gridRenderer":{"header":{"gridHeaderRenderer":{"title":{"runs":[{"text":"Browse"}]}}},
              "items":[
                {"musicNavigationButtonRenderer":{"buttonText":{"runs":[{"text":"Charts"}]},
                  "clickCommand":{"browseEndpoint":{"browseId":"FEmusic_charts"}}}},
                {"musicNavigationButtonRenderer":{"buttonText":{"runs":[{"text":"Chill"}]},
                  "clickCommand":{"browseEndpoint":{"browseId":"FEmusic_moods_and_genres_category","params":"ggMPOg1u"}}}}]}}}""",
        )
        val shelves = parseShelves(chips)
        assertEquals(listOf("Browse"), shelves.map { it.title })
        assertEquals(listOf("FEmusic_charts", "FEmusic_moods_and_genres_category#ggMPOg1u"), shelves[0].items.map { it.browseId })
        assertEquals(
            listOf("abba dancing queen"),
            parseSuggestions(
                j(
                    """{"contents":[{"searchSuggestionRenderer":{"suggestion":{"runs":[{"text":"abba "},{"text":"dancing queen"}]}}},
                     {"searchSuggestionRenderer":{"suggestion":{"runs":[{"text":""}]}}}]}""",
                ),
            ),
        )
    }

    @Test
    fun signedInHeaders() {
        val cookies = mapOf("SAPISID" to "abc", "__Secure-3PAPISID" to "abc", "SID" to "s", "YSC" to "y")
        assertTrue(Auth.isSignedIn(cookies) && !Auth.isSignedIn(mapOf("YSC" to "y")))
        val h = "1700000000_27b236f59d4ec583d7530f2c7055d2f9c6aecf92"
        assertEquals("SAPISIDHASH $h SAPISID3PHASH $h", authorization(cookies, "https://www.youtube.com", 1_700_000_000))
        assertEquals("SAPISID=abc; SID=s; YSC=y; __Secure-3PAPISID=abc", Auth(cookies).headers("https://music.youtube.com")["Cookie"])
        assertTrue(Auth().headers("x").isEmpty())
        val rotated = mutableListOf<Map<String, String>>()
        val auth = Auth(cookies) { rotated += it }
        auth.absorb(Response(200, mapOf("Set-Cookie" to listOf("SID=new; Path=/; Secure", "OTHER=x")), ByteArray(0)))
        assertEquals("new", rotated.single()["SID"])
        assertTrue("OTHER" !in rotated.single())
    }

    @Test
    fun playerScript() {
        assertEquals(
            "https://www.youtube.com/s/player/ec429320/player_ias.vflset/en_US/base.js",
            ChallengeSolver.playerUrl(""""x "jsUrl":"/s/player/ec429320/player_es6_tcc.vflset/en_US/base.js","y""""),
        )
        assertEquals(20367, ChallengeSolver.signatureTimestampIn("a,signatureTimestamp:20367,b"))
        assertTrue(ChallengeSolver.script.contains("Object.assign(globalThis, lib)"))
    }

    @Test
    fun networkFailures() {
        assertTrue(java.net.UnknownHostException("music.youtube.com").isNetworkFailure())
        assertTrue(java.net.SocketTimeoutException("timeout").isNetworkFailure())
        assertTrue(java.net.SocketTimeoutException("timeout").isTimeout())
        assertTrue(!YTMError.Http("x", 500).isNetworkFailure())
        assertTrue(!YTMError.NoStream.isNetworkFailure())
    }
}

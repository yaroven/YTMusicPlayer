package dev.ytmplayer.core

import kotlinx.coroutines.runBlocking
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

/** The live half of YTMKitChecks: `YTM_LIVE=1 ./gradlew :core:test`. */
class LiveTest {
    private val live = System.getenv("YTM_LIVE") == "1"

    @Test
    fun catalog() = runBlocking {
        if (!live) return@runBlocking
        val catalog = Catalog()
        val found = catalog.search("abba dancing queen", SearchKind.Songs)
        println("search: ${found.size} songs, first ${found.firstOrNull()?.title} by ${found.firstOrNull()?.track?.artist}")
        assertTrue(found.isNotEmpty())
        val home = catalog.home()
        println("home: ${home.size} shelves")
        assertTrue(home.isNotEmpty())
        val radio = catalog.radio("YkLLcIKhJ64")
        println("radio: ${radio.size} tracks")
        assertTrue(radio.size > 10)
        val artist = catalog.artistId("YkLLcIKhJ64")!!
        val page = catalog.page(artist)
        println("artist: ${page.title}, ${page.tracks.size} songs, ${page.shelves.size} shelves")
        assertTrue(page.tracks.size > 5)
        val explore = catalog.explore()
        println("explore: ${explore.size} shelves")
        assertTrue(explore.size > 2)
        val chip = explore.flatMap { it.items }.first { it.browseId.startsWith("FEmusic_") }
        assertTrue(catalog.page(chip.browseId).shelves.isNotEmpty(), "chip opens as a page")
        val suggested = catalog.suggestions("abba danc")
        println("suggestions: ${suggested.take(3)}")
        assertTrue(suggested.any { it.contains("dancing") })
        assertTrue(catalog.search("eminem", SearchKind.Songs).any { it.explicit }, "E badges")
        assertTrue(catalog.related("YkLLcIKhJ64").isNotEmpty(), "related")
    }

    @Test
    fun lyrics() = runBlocking {
        if (!live) return@runBlocking
        val lyrics = LyricsFinder(Catalog()).find(Track("fJ9rUzIMcZQ", "Bohemian Rhapsody", "Queen"), 359.0)
        println("lyrics: ${lyrics?.lines?.size} lines from ${lyrics?.source}, synced ${lyrics?.synced}")
        assertEquals(true, lyrics?.synced)
    }

    @Test
    fun stream() = runBlocking {
        if (!live) return@runBlocking
        val start = System.currentTimeMillis()
        val stream = Resolver().stream("YkLLcIKhJ64")
        println("stream: ${(System.currentTimeMillis() - start) / 1000.0} s, ${stream.contentLength} bytes, ${stream.duration} s, loudness ${stream.loudnessDb}")
        assertTrue(stream.duration != null)
        // Every range must work (a PO-token-bound URL fails past the first MB).
        val length = stream.contentLength ?: 1_000_000
        for (offset in listOf(0L, length / 2, maxOf(0, length - 70_000))) {
            val r = Http.get(stream.url, mapOf("Range" to "bytes=$offset-${offset + 65_535}", "User-Agent" to stream.userAgent))
            println("  range $offset: ${r.status}, ${r.body.size} bytes")
            assertEquals(206, r.status)
        }
        // What ExoPlayer asks first: an open-ended range from 0.
        val open = Http.get(stream.url, mapOf("Range" to "bytes=0-", "User-Agent" to "ExoPlayerLib/1.5.1"))
        println("  open range: ${open.status}, ${open.body.size} bytes")
        assertTrue(open.status == 206 || open.status == 200)
    }
}

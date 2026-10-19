package dev.ytmplayer.core

import kotlinx.coroutines.runBlocking
import kotlin.test.Test

class TimingTest {
    @Test
    fun timing() = runBlocking {
        if (System.getenv("YTM_LIVE") != "1") return@runBlocking
        val c = Catalog()
        val first = c.call("browse", json("browseId" to "VLPL0GvsLQil0MmYC96KEs_7dTNsLm1PS6JX"))
        val shelf = findFirst(first, "musicPlaylistShelfRenderer")
        println("TIMING shelf conts ${findAll(shelf, "continuationCommand").size} all ${findAll(first, "continuationCommand").size}")
        val token = (findFirst(shelf, "continuationCommand") as org.json.JSONObject).getString("token")
        var t = System.currentTimeMillis()
        val more = c.call("browse", json("continuation" to token))
        println("TIMING more ${System.currentTimeMillis() - t}ms songs ${songsIn(more).size} keys ${more.keys().asSequence().toList()} next ${findAll(more, "continuationCommand").size}")
        val tok2 = (findFirst(more, "continuationCommand") as? org.json.JSONObject)?.getString("token")
        if (tok2 != null) { t = System.currentTimeMillis(); val m2 = c.call("browse", json("continuation" to tok2)); println("TIMING more2 ${System.currentTimeMillis() - t}ms songs ${songsIn(m2).size} next ${findAll(m2, "continuationCommand").size}") }
    }
}

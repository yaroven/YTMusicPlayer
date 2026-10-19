package dev.ytmplayer.android.ui

import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.ui.Modifier
import androidx.compose.ui.input.pointer.pointerInput
import dev.ytmplayer.android.App
import dev.ytmplayer.core.Item
import dev.ytmplayer.core.Page
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Deferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.async

/**
 * Album, artist and playlist pages: the last ones opened (to open again at
 * once), the ones loading since a finger touched their card (prefetch),
 * and the cards themselves (so a page shows its title and art before it
 * has loaded). Main thread only.
 */
object PageCache {
    private const val SIZE = 12
    private const val PREVIEWS = 300

    private val pages = LinkedHashMap<String, Page>()
    private val loading = HashMap<String, Deferred<Page>>()
    private val previews = HashMap<String, Item>()
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)

    fun cached(id: String): Page? = pages[id]

    fun preview(id: String): Item? = previews[id]

    /** A card or row is on screen: its title and art are what its page shows first. */
    fun note(item: Item) {
        if (item.track != null || previews.containsKey(item.browseId)) return
        if (previews.size >= PREVIEWS) previews.clear()
        previews[item.browseId] = item
    }

    /** A finger is down on a card: start loading its page. */
    fun prefetch(item: Item) {
        if (item.track != null) return
        note(item)
        if (cached(item.browseId) == null && item.browseId !in loading) start(item.browseId)
    }

    /** The page's first answer: from here, from a prefetch under way, or from YouTube Music. */
    suspend fun load(id: String): Page = cached(id) ?: (loading[id] ?: start(id)).await()

    /** Keeps `page` (also when more of its songs arrived). */
    fun remember(id: String, page: Page) {
        pages.remove(id)
        pages[id] = page
        while (pages.size > SIZE) pages.remove(pages.keys.first())
    }

    private fun start(id: String): Deferred<Page> {
        val job = scope.async { App.instance.catalog.page(id, full = false) }
        loading[id] = job
        job.invokeOnCompletion { error ->
            scope.async {
                loading.remove(id)
                if (error == null) remember(id, job.getCompleted())
            }
        }
        return job
    }
}

/** Starts loading the card's page as soon as a finger is down on it (before the tap ends). */
fun Modifier.prefetchOnPress(item: Item): Modifier =
    if (item.track != null) {
        this
    } else {
        pointerInput(item.browseId) {
            awaitEachGesture {
                awaitFirstDown(requireUnconsumed = false)
                PageCache.prefetch(item)
            }
        }
    }

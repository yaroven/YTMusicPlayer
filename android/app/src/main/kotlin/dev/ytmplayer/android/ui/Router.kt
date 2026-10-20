package dev.ytmplayer.android.ui

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import dev.ytmplayer.android.App
import dev.ytmplayer.android.LocalList
import dev.ytmplayer.core.SearchKind
import dev.ytmplayer.core.Track
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch

enum class Tab { Home, Explore, Library }

sealed interface Route {
    /** Album, artist or playlist page by browse id. */
    data class Page(val id: String) : Route
    data class List(val list: LocalList) : Route
    object AgeTest : Route
    object Settings : Route
    /** Search, opened from the magnifier in a screen's header. */
    object Search : Route
}

/** Each tab's back stack, so "Go to artist" works from anywhere. */
object Router {
    var tab by mutableStateOf(Tab.Home)
    private val stacks = Tab.entries.associateWith { mutableStateListOf<Route>() }
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)

    fun stack(tab: Tab): List<Route> = stacks.getValue(tab)

    fun open(route: Route) {
        App.instance.player.showNowPlaying = false
        stacks.getValue(tab).add(route)
    }

    /** Back: closes the top screen; false when there is none. */
    fun back(): Boolean {
        val stack = stacks.getValue(tab)
        if (stack.isEmpty()) return false
        stack.removeAt(stack.lastIndex)
        return true
    }

    fun select(target: Tab) {
        // Tapping the open tab again goes to its root.
        if (target == tab) stacks.getValue(tab).clear()
        tab = target
    }

    fun openDownloads() {
        tab = Tab.Library
        open(Route.List(LocalList.Downloads))
    }

    /** The song's artist page, as linked under the song, else the first artist search hit. */
    fun openArtist(track: Track) {
        val catalog = App.instance.catalog
        scope.launch {
            runCatching { catalog.artistId(track.videoId) }.getOrNull()?.let { return@launch open(Route.Page(it)) }
            val name = track.artist.substringBefore(',')
            runCatching { catalog.search(name, SearchKind.Artists).firstOrNull() }.getOrNull()
                ?.let { open(Route.Page(it.browseId)) }
                ?: Notice.show(App.instance.getString(dev.ytmplayer.android.R.string.artist_not_found))
        }
    }
}

/** One-line messages at the bottom (a snackbar). */
object Notice {
    var text by mutableStateOf<String?>(null)
        private set
    private var serial = 0
    var id by mutableStateOf(0)
        private set

    fun show(message: String) {
        text = message
        id = ++serial
    }

    fun clear() {
        text = null
    }
}

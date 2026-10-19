package dev.ytmplayer.android.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.Bookmark
import androidx.compose.material.icons.filled.BookmarkBorder
import androidx.compose.material.icons.filled.Clear
import androidx.compose.material.icons.filled.CloudOff
import androidx.compose.material.icons.filled.PlayArrow
import androidx.compose.material.icons.filled.Search
import androidx.compose.material.icons.filled.SearchOff
import androidx.compose.material.icons.filled.Shuffle
import androidx.compose.material3.Button
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilterChip
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import dev.ytmplayer.android.App
import dev.ytmplayer.android.R
import dev.ytmplayer.core.ItemKind
import dev.ytmplayer.core.Page
import dev.ytmplayer.core.SearchKind
import dev.ytmplayer.core.Shelf
import dev.ytmplayer.core.Track
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun TopBar(title: String, back: Boolean = false, actions: @Composable () -> Unit = {}) {
    TopAppBar(
        title = { Text(title, maxLines = 1) },
        navigationIcon = {
            if (back) IconButton(onClick = { Router.back() }) { Icon(Icons.AutoMirrored.Filled.ArrowBack, stringResource(R.string.back)) }
        },
        actions = { actions() },
        colors = TopAppBarDefaults.topAppBarColors(containerColor = Color.Transparent),
        // The scaffold already keeps clear of the status bar.
        windowInsets = WindowInsets(0),
    )
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun HomeScreen() {
    val app = App.instance
    val library = app.library
    val signedIn = app.account.signedIn
    val online by app.network.online.collectAsState()
    // Signed in, Home is the account's own.
    val home = rememberLoad(signedIn) { app.catalog.home() }
    Column(Modifier.fillMaxSize()) {
        TopBar(stringResource(R.string.home))
        PullToRefreshBox(home.loading && home.value != null, onRefresh = home::retry, Modifier.fillMaxSize()) {
            val history = library.history.take(20).map { it.toItem() }
            val shelves = home.value
            if (shelves == null && history.isEmpty()) {
                Loaded(home) {}
                return@PullToRefreshBox
            }
            LazyColumn(contentPadding = PaddingValues(bottom = 16.dp), verticalArrangement = Arrangement.spacedBy(20.dp)) {
                if (!online && library.downloads.isNotEmpty()) {
                    item { OfflineHint() }
                }
                if (history.isNotEmpty()) item { ShelfView(Shelf(stringResource(R.string.listen_again), history)) }
                when {
                    shelves != null -> items(shelves, key = { it.id }) { ShelfView(it) }
                    home.error != null && !home.loading -> item { ErrorState(home.error!!, Modifier.height(320.dp), home::retry) }
                    else -> item { Loading(Modifier.height(200.dp)) }
                }
            }
        }
    }
}

/** Offline with songs on the phone: a way to them. */
@Composable
private fun OfflineHint() {
    Row(
        Modifier.fillMaxWidth().padding(horizontal = 16.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(Icons.Default.CloudOff, null, tint = MaterialTheme.colorScheme.onSurfaceVariant)
        Spacer(Modifier.width(12.dp))
        Text(
            stringResource(R.string.offline_downloads_hint), Modifier.weight(1f),
            style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        TextButton(onClick = { Router.openDownloads() }) { Text(stringResource(R.string.open_downloads)) }
    }
}

fun Track.toItem() = dev.ytmplayer.core.Item(ItemKind.Song, videoId, title, artist, track = this, explicit = explicit)

/** Search, with Explore when nothing is typed and YouTube Music's suggestions while typing. */
@Composable
fun SearchScreen() {
    val app = App.instance
    var query by rememberSaveable { mutableStateOf("") }
    var searched by rememberSaveable { mutableStateOf("") }
    var kind by rememberSaveable { mutableStateOf(SearchKind.Songs) }
    var suggestions by remember { mutableStateOf<List<String>>(emptyList()) }
    val scope = rememberCoroutineScope()
    val keyboard = LocalSoftwareKeyboardController.current

    // One request after the typing stops, not one per keystroke.
    LaunchedEffect(query) {
        val text = query.trim()
        if (text.isEmpty() || text == searched) {
            suggestions = emptyList()
            return@LaunchedEffect
        }
        delay(250)
        suggestions = runCatching { app.catalog.suggestions(text) }.getOrDefault(emptyList()).take(8)
    }
    fun run(text: String = query) {
        val q = text.trim()
        if (q.isEmpty()) return
        query = q
        searched = q
        suggestions = emptyList()
        keyboard?.hide()
    }

    Column(Modifier.fillMaxSize()) {
        TopBar(stringResource(R.string.search))
        OutlinedTextField(
            query, { query = it },
            Modifier.fillMaxWidth().padding(horizontal = 16.dp),
            placeholder = { Text(stringResource(R.string.search_prompt)) },
            leadingIcon = { Icon(Icons.Default.Search, null) },
            trailingIcon = {
                if (query.isNotEmpty()) {
                    IconButton(onClick = {
                        query = ""
                        searched = ""
                    }) { Icon(Icons.Default.Clear, stringResource(R.string.clear)) }
                }
            },
            singleLine = true,
            keyboardOptions = KeyboardOptions(imeAction = ImeAction.Search),
            keyboardActions = KeyboardActions(onSearch = { run() }),
        )
        Spacer(Modifier.height(8.dp))
        when {
            suggestions.isNotEmpty() && query.trim() != searched -> LazyColumn {
                items(suggestions) { s ->
                    Row(
                        Modifier.fillMaxWidth().clickable { scope.launch { run(s) } }.padding(horizontal = 20.dp, vertical = 14.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        Icon(Icons.Default.Search, null, tint = MaterialTheme.colorScheme.onSurfaceVariant)
                        Spacer(Modifier.width(16.dp))
                        Text(s)
                    }
                }
            }
            searched.isEmpty() -> Explore()
            else -> Results(searched, kind) { kind = it }
        }
    }
}

@Composable
private fun Explore() {
    val explore = rememberLoad(Unit) { App.instance.catalog.explore() }
    Loaded(explore) { shelves ->
        LazyColumn(contentPadding = PaddingValues(bottom = 16.dp), verticalArrangement = Arrangement.spacedBy(20.dp)) {
            items(shelves, key = { it.id }) { ShelfView(it) }
        }
    }
}

@Composable
private fun Results(query: String, kind: SearchKind, onKind: (SearchKind) -> Unit) {
    val results = rememberLoad(query to kind) { App.instance.catalog.search(query, kind) }
    Column {
        LazyRow(contentPadding = PaddingValues(horizontal = 16.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            items(SearchKind.entries) { k ->
                FilterChip(k == kind, onClick = { onKind(k) }, label = { Text(kindLabel(k)) })
            }
        }
        Loaded(results) { found ->
            if (found.isEmpty()) {
                Message(Icons.Default.SearchOff, stringResource(R.string.no_results, query))
                return@Loaded
            }
            LazyColumn(contentPadding = PaddingValues(bottom = 16.dp)) {
                if (kind == SearchKind.Songs) {
                    val tracks = found.mapNotNull { it.track }
                    itemsIndexed(tracks) { i, t -> TrackRow(t, tracks, i) }
                } else {
                    items(found, key = { it.id }) { ItemRow(it) }
                }
            }
        }
    }
}

@Composable
private fun kindLabel(kind: SearchKind): String = stringResource(
    when (kind) {
        SearchKind.Songs -> R.string.songs
        SearchKind.Albums -> R.string.albums
        SearchKind.Artists -> R.string.artists
        SearchKind.Playlists -> R.string.playlists
    },
)

@Composable
fun PageScreen(id: String) {
    val app = App.instance
    val page = rememberLoad(id) { app.catalog.page(id) }
    var showAll by rememberSaveable(id) { mutableStateOf(false) }
    Column(Modifier.fillMaxSize()) {
        val loaded = page.value
        TopBar(loaded?.title ?: "", back = true) {
            if (loaded != null) {
                val saved = app.library.isSaved(loaded.id)
                IconButton(onClick = { app.library.toggleSaved(loaded) }) {
                    Icon(if (saved) Icons.Default.Bookmark else Icons.Default.BookmarkBorder, stringResource(R.string.save))
                }
            }
        }
        Loaded(page) { p -> PageContent(p, showAll) { showAll = true } }
    }
}

@Composable
private fun PageContent(page: Page, showAll: Boolean, onShowAll: () -> Unit) {
    val shown = if (page.kind == ItemKind.Artist && !showAll) page.tracks.take(TOP_SONGS) else page.tracks
    LazyColumn(contentPadding = PaddingValues(bottom = 24.dp)) {
        item {
            Column(Modifier.fillMaxWidth().padding(16.dp), horizontalAlignment = Alignment.CenterHorizontally) {
                Artwork(page.thumbnail?.let { Art.Url(it) } ?: Art.None, 220.dp, round = page.kind == ItemKind.Artist)
                Spacer(Modifier.height(12.dp))
                Text(page.title, style = MaterialTheme.typography.headlineSmall, fontWeight = FontWeight.Bold, textAlign = TextAlign.Center)
                Text(
                    page.subtitle, style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant, textAlign = TextAlign.Center,
                )
                Spacer(Modifier.height(12.dp))
                PlayButtons(page.tracks)
            }
        }
        if (page.kind == ItemKind.Artist && page.tracks.isNotEmpty()) {
            item { SectionTitle(stringResource(R.string.top_songs)) }
        }
        itemsIndexed(shown) { i, t -> TrackRow(t, page.tracks, i, number = if (page.kind == ItemKind.Album) i + 1 else null) }
        if (shown.size < page.tracks.size) {
            item {
                TextButton(onClick = onShowAll, Modifier.padding(horizontal = 8.dp)) {
                    Text(stringResource(R.string.show_all_songs, page.tracks.size))
                }
            }
        }
        items(page.shelves, key = { it.id }) { shelf ->
            Spacer(Modifier.height(16.dp))
            ShelfView(shelf)
        }
    }
}

@Composable
fun PlayButtons(tracks: List<Track>) {
    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
        Button(onClick = { playFrom(tracks, 0) }, Modifier.weight(1f), enabled = tracks.isNotEmpty()) {
            Icon(Icons.Default.PlayArrow, null)
            Spacer(Modifier.width(6.dp))
            Text(stringResource(R.string.play))
        }
        OutlinedButton(onClick = { playFrom(tracks.shuffled(), 0) }, Modifier.weight(1f), enabled = tracks.isNotEmpty()) {
            Icon(Icons.Default.Shuffle, null)
            Spacer(Modifier.width(6.dp))
            Text(stringResource(R.string.shuffle))
        }
    }
}

@Composable
fun SectionTitle(text: String) {
    Text(
        text, style = MaterialTheme.typography.titleSmall, color = MaterialTheme.colorScheme.primary,
        modifier = Modifier.padding(start = 16.dp, end = 16.dp, top = 16.dp, bottom = 4.dp),
    )
}

@Composable
fun songsCount(n: Int): String = pluralStringResource(R.plurals.songs, n, n)

/** Which client plays an age-restricted video with this account. */
@Composable
fun AgeTestScreen() {
    val app = App.instance
    var videoId by rememberSaveable { mutableStateOf("Tq92D6wQ1mg") }
    var lines by remember { mutableStateOf<List<String>>(emptyList()) }
    var running by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()
    Column(Modifier.fillMaxSize()) {
        TopBar(stringResource(R.string.age_test), back = true)
        Column(Modifier.padding(16.dp)) {
            OutlinedTextField(videoId, { videoId = it }, Modifier.fillMaxWidth(), label = { Text(stringResource(R.string.video_id)) }, singleLine = true)
            Spacer(Modifier.height(8.dp))
            Button(
                enabled = !running,
                onClick = {
                    running = true
                    lines = emptyList()
                    scope.launch {
                        lines = app.resolver.diagnose(videoIdIn(videoId))
                        running = false
                    }
                },
            ) { Text(stringResource(if (running) R.string.testing else R.string.run_test)) }
            Spacer(Modifier.height(8.dp))
            Text(stringResource(R.string.age_test_footer), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            Spacer(Modifier.height(12.dp))
            lines.forEach { Text(it, style = MaterialTheme.typography.bodySmall) }
        }
    }
}

/** "https://music.youtube.com/watch?v=ID&list=…" or "youtu.be/ID" → "ID". */
fun videoIdIn(text: String): String {
    val t = text.trim()
    Regex("[?&]v=([^&]+)").find(t)?.let { return it.groupValues[1] }
    Regex("youtu\\.be/([^?&/]+)").find(t)?.let { return it.groupValues[1] }
    return t
}

private const val TOP_SONGS = 5

@Composable
fun Centered(content: @Composable () -> Unit) {
    Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) { content() }
}

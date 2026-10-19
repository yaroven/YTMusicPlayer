package dev.ytmplayer.android.ui

import androidx.compose.animation.core.RepeatMode as AnimRepeat
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.foundation.clickable
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.material.icons.filled.NorthWest
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.runtime.derivedStateOf
import androidx.compose.ui.draw.clip
import androidx.compose.ui.unit.sp
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.background
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
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.statusBarsPadding
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
        windowInsets = WindowInsets.statusBars,
    )
}

/**
 * YouTube Music's Home: the logo and the account at the top, mood chips,
 * then shelves ("Quick picks" as a grid of tiles, videos wide, the rest as
 * cards), more of them loading as you scroll, over the colours of the first
 * picture.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun HomeScreen() {
    val app = App.instance
    val library = app.library
    val signedIn = app.account.signedIn
    val online by app.network.online.collectAsState()
    var chip by remember { mutableStateOf<dev.ytmplayer.core.HomeChip?>(null) }
    var chips by remember { mutableStateOf<List<dev.ytmplayer.core.HomeChip>>(emptyList()) }
    // Signed in, Home is the account's own; a chip is Home for that mood.
    val home = rememberLoad(signedIn to chip?.params) { app.catalog.homeFeed(chip?.params) }
    var more by remember(home.value) { mutableStateOf<List<Shelf>>(emptyList()) }
    var token by remember(home.value) { mutableStateOf(home.value?.continuation) }
    LaunchedEffect(home.value) { home.value?.chips?.takeIf { it.isNotEmpty() && chip == null }?.let { chips = it } }
    val list = rememberLazyListState()
    // The next shelves when the end of the list comes near.
    val nearEnd by remember { derivedStateOf { list.layoutInfo.visibleItemsInfo.lastOrNull()?.index ?: 0 >= list.layoutInfo.totalItemsCount - 2 } }
    LaunchedEffect(nearEnd, token) {
        val t = token ?: return@LaunchedEffect
        if (!nearEnd) return@LaunchedEffect
        val (shelves, next) = runCatching { app.catalog.moreHome(t, home.value?.visitor) }.getOrNull() ?: return@LaunchedEffect
        more = more + shelves
        token = next
    }
    val shelves = home.value?.shelves.orEmpty() + more
    Box(Modifier.fillMaxSize()) {
        AmbientBackdrop(shelves.firstOrNull()?.items?.firstOrNull()?.let { it.track?.thumbnailUrl ?: it.thumbnail })
        PullToRefreshBox(home.loading && home.value != null, onRefresh = home::retry, Modifier.fillMaxSize()) {
            val history = library.history.take(20).map { it.toItem() }
            LazyColumn(state = list, contentPadding = PaddingValues(bottom = 24.dp), verticalArrangement = Arrangement.spacedBy(28.dp)) {
                item {
                    Column {
                        Row(Modifier.fillMaxWidth().statusBarsPadding().padding(horizontal = 16.dp, vertical = 12.dp), verticalAlignment = Alignment.CenterVertically) {
                            YTLogo()
                            Spacer(Modifier.weight(1f))
                            AvatarButton()
                        }
                        if (chips.isNotEmpty()) {
                            LazyRow(contentPadding = PaddingValues(horizontal = 16.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                                items(chips, key = { it.params }) { c ->
                                    ChipButton(c.title, chip == c) { chip = if (chip == c) null else c }
                                }
                            }
                        }
                    }
                }
                if (!online && library.downloads.isNotEmpty()) item { OfflineHint() }
                if (chip == null && history.isNotEmpty()) item { ShelfView(Shelf(stringResource(R.string.listen_again), history)) }
                when {
                    home.value != null -> items(shelves, key = { it.id }) { ShelfView(it) }
                    home.error != null && !home.loading -> item { ErrorState(home.error!!, Modifier.height(320.dp), home::retry) }
                    else -> item { Loading(Modifier.height(240.dp)) }
                }
                if (token != null) item { Box(Modifier.fillMaxWidth(), contentAlignment = Alignment.Center) { CircularProgressIndicator(Modifier.size(28.dp)) } }
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

    Column(Modifier.fillMaxSize().statusBarsPadding()) {
        // YouTube Music's search bar: a dark rounded field.
        Row(
            Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 10.dp).height(44.dp)
                .clip(RoundedCornerShape(22.dp)).background(YT.surface).padding(horizontal = 14.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Icon(Icons.Default.Search, null, tint = YT.secondary)
            Spacer(Modifier.width(10.dp))
            Box(Modifier.weight(1f)) {
                if (query.isEmpty()) Text(stringResource(R.string.search_prompt), color = YT.secondary, fontSize = 16.sp)
                androidx.compose.foundation.text.BasicTextField(
                    query, { query = it }, Modifier.fillMaxWidth(), singleLine = true,
                    textStyle = androidx.compose.ui.text.TextStyle(color = Color.White, fontSize = 16.sp),
                    cursorBrush = androidx.compose.ui.graphics.SolidColor(Color.White),
                    keyboardOptions = KeyboardOptions(imeAction = ImeAction.Search),
                    keyboardActions = KeyboardActions(onSearch = { run() }),
                )
            }
            if (query.isNotEmpty()) {
                IconButton(onClick = {
                    query = ""
                    searched = ""
                }) { Icon(Icons.Default.Clear, stringResource(R.string.clear), tint = YT.secondary) }
            }
        }
        when {
            suggestions.isNotEmpty() && query.trim() != searched -> LazyColumn {
                items(suggestions) { s ->
                    Row(
                        Modifier.fillMaxWidth().clickable { scope.launch { run(s) } }.padding(horizontal = 20.dp, vertical = 14.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        Icon(Icons.Default.Search, null, tint = YT.secondary)
                        Spacer(Modifier.width(16.dp))
                        Text(s, Modifier.weight(1f))
                        Icon(Icons.Default.NorthWest, null, tint = YT.secondary)
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
        LazyColumn(contentPadding = PaddingValues(top = 8.dp, bottom = 16.dp), verticalArrangement = Arrangement.spacedBy(28.dp)) {
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
                ChipButton(kindLabel(k), k == kind) { onKind(k) }
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
    // The first answer shows at once; a long playlist's other songs (or an
    // artist's full top songs) arrive behind it.
    // Opened lately, or prefetched while the finger was down: at once.
    val page = rememberLoad(id) { PageCache.load(id) }
    var showAll by rememberSaveable(id) { mutableStateOf(false) }
    LaunchedEffect(page.value?.id) {
        var p = page.value ?: return@LaunchedEffect
        p.allSongsId?.let { all ->
            val list = runCatching { app.catalog.page(all) }.getOrNull()
            if (list != null && list.tracks.size > p.tracks.size) {
                p = p.copy(tracks = list.tracks)
            }
            p = p.copy(allSongsId = null)
            page.update(p)
            PageCache.remember(id, p)
        }
        var parts = 0
        while (parts++ < 50) {
            val token = p.continuation ?: break
            val (more, next) = runCatching { app.catalog.moreTracks(token) }.getOrNull() ?: break
            p = p.copy(tracks = p.tracks + more, continuation = next)
            page.update(p)
            PageCache.remember(id, p)
        }
    }
    val loaded = page.value
    val preview = PageCache.preview(id)
    Box(Modifier.fillMaxSize()) {
        AmbientBackdrop(loaded?.thumbnail ?: preview?.thumbnail, 520.dp)
        Column(Modifier.fillMaxSize()) {
        TopBar("", back = true)
        when {
            loaded != null -> PageContent(loaded, preview?.title.orEmpty(), showAll) { showAll = true }
            // The card's title and art at once, the songs when they come.
            preview != null -> PagePreview(preview, page)
            else -> Loaded(page) {}
        }
        }
    }
}

@Composable
private fun PagePreview(item: dev.ytmplayer.core.Item, page: Load<Page>) {
    LazyColumn(contentPadding = PaddingValues(bottom = 24.dp)) {
        item {
            PageHeader(item.title, item.subtitle, item.thumbnail, item.kind == ItemKind.Artist, emptyList(), null)
        }
        val error = page.error
        if (error != null && !page.loading) {
            item { ErrorState(error, Modifier.height(320.dp), page::retry) }
        } else {
            items(8) { SkeletonRow() }
        }
    }
}

/** A grey placeholder row while songs load. */
@Composable
private fun SkeletonRow() {
    val pulse = rememberInfiniteTransition(label = "skeleton")
    val alpha by pulse.animateFloat(
        initialValue = 0.06f,
        targetValue = 0.14f,
        animationSpec = infiniteRepeatable(tween(800), AnimRepeat.Reverse),
        label = "alpha",
    )
    val shade = Color.White.copy(alpha = alpha)
    Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 6.dp), verticalAlignment = Alignment.CenterVertically) {
        Box(Modifier.size(48.dp).background(shade, RoundedCornerShape(5.dp)))
        Spacer(Modifier.width(12.dp))
        Column {
            Box(Modifier.width(180.dp).height(12.dp).background(shade, RoundedCornerShape(6.dp)))
            Spacer(Modifier.height(6.dp))
            Box(Modifier.width(110.dp).height(10.dp).background(shade, RoundedCornerShape(5.dp)))
        }
    }
}

@Composable
private fun PageContent(page: Page, cardTitle: String, showAll: Boolean, onShowAll: () -> Unit) {
    val shown = if (page.kind == ItemKind.Artist && !showAll) page.tracks.take(TOP_SONGS) else page.tracks
    LazyColumn(contentPadding = PaddingValues(bottom = 24.dp)) {
        // Explore categories ("Chill") are only shelves: no art, no buttons.
        if (page.tracks.isEmpty() && page.thumbnail == null) {
            item {
                Text(
                    page.title.ifEmpty { cardTitle },
                    Modifier.padding(16.dp), fontSize = 28.sp, fontWeight = FontWeight.Bold,
                )
            }
        } else item {
            PageHeader(page.title.ifEmpty { cardTitle }, page.subtitle, page.thumbnail, page.kind == ItemKind.Artist, page.tracks, page)
        }
        if (page.kind == ItemKind.Artist && page.tracks.isNotEmpty()) {
            item {
                Text(stringResource(R.string.top_songs), Modifier.padding(horizontal = 16.dp, vertical = 8.dp), fontSize = 22.sp, fontWeight = FontWeight.Bold)
            }
        }
        itemsIndexed(shown) { i, t -> TrackRow(t, page.tracks, i, number = if (page.kind == ItemKind.Album) i + 1 else null) }
        if (shown.size < page.tracks.size) {
            item {
                Box(Modifier.fillMaxWidth().padding(8.dp), contentAlignment = Alignment.Center) {
                    OutlinePill(stringResource(R.string.show_all_songs, page.tracks.size), onShowAll)
                }
            }
        }
        items(page.shelves, key = { it.id }) { shelf ->
            Spacer(Modifier.height(28.dp))
            ShelfView(shelf)
        }
    }
}

/** Art, title, subtitle, then save / play / shuffle (also before the songs are in). */
@Composable
private fun PageHeader(title: String, subtitle: String, thumbnail: String?, round: Boolean, tracks: List<Track>, page: Page?) {
    val library = App.instance.library
    Column(Modifier.fillMaxWidth().padding(16.dp), horizontalAlignment = Alignment.CenterHorizontally) {
        Artwork(thumbnail?.let { Art.Url(it) } ?: Art.None, if (round) 200.dp else 250.dp, round = round)
        Spacer(Modifier.height(16.dp))
        Text(title, fontSize = 26.sp, fontWeight = FontWeight.Bold, textAlign = TextAlign.Center)
        if (subtitle.isNotEmpty()) {
            Spacer(Modifier.height(4.dp))
            Text(subtitle, fontSize = 14.sp, color = YT.secondary, textAlign = TextAlign.Center)
        }
        Spacer(Modifier.height(16.dp))
        Row(horizontalArrangement = Arrangement.spacedBy(28.dp), verticalAlignment = Alignment.CenterVertically) {
            val saved = page != null && library.isSaved(page.id)
            CircleIcon(
                if (saved) Icons.Default.Bookmark else Icons.Default.BookmarkBorder, stringResource(R.string.save),
                enabled = page != null,
            ) { page?.let(library::toggleSaved) }
            CircleIcon(Icons.Default.PlayArrow, stringResource(R.string.play), 64.dp, filled = true, enabled = tracks.isNotEmpty()) {
                playFrom(tracks, 0)
            }
            CircleIcon(Icons.Default.Shuffle, stringResource(R.string.shuffle), enabled = tracks.isNotEmpty()) {
                playFrom(tracks.shuffled(), 0)
            }
        }
    }
}

/** Play and shuffle for a list (liked songs, a playlist…). */
@Composable
fun PlayButtons(tracks: List<Track>) {
    Row(horizontalArrangement = Arrangement.spacedBy(20.dp), verticalAlignment = Alignment.CenterVertically) {
        CircleIcon(Icons.Default.PlayArrow, stringResource(R.string.play), 56.dp, filled = true, enabled = tracks.isNotEmpty()) {
            playFrom(tracks, 0)
        }
        CircleIcon(Icons.Default.Shuffle, stringResource(R.string.shuffle), enabled = tracks.isNotEmpty()) {
            playFrom(tracks.shuffled(), 0)
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

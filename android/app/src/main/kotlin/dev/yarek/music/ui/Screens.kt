package dev.yarek.music.ui

import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.ui.graphics.Brush
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.material.icons.filled.Sensors
import androidx.compose.material.icons.filled.Share
import androidx.compose.ui.platform.LocalContext
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.ui.focus.focusRequester
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.DropdownMenu
import androidx.compose.material.icons.filled.MoreVert
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
import dev.yarek.music.App
import dev.yarek.music.R
import dev.yarek.core.ItemKind
import dev.yarek.core.Page
import dev.yarek.core.SearchKind
import dev.yarek.core.Shelf
import dev.yarek.core.Track
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
/** Home's and Explore's header: the logo, search and the account. */
@Composable
fun HomeHeader() {
    Row(Modifier.fillMaxWidth().statusBarsPadding().padding(start = 16.dp, end = 8.dp, top = 4.dp).height(48.dp), verticalAlignment = Alignment.CenterVertically) {
        YTLogo()
        Spacer(Modifier.weight(1f))
        IconButton(onClick = { Router.open(Route.Search) }) { Icon(Icons.Default.Search, stringResource(R.string.search)) }
        Box(Modifier.padding(horizontal = 8.dp)) { AvatarButton() }
    }
}

@OptIn(ExperimentalMaterial3Api::class, ExperimentalFoundationApi::class)
@Composable
fun HomeScreen() {
    val app = App.instance
    val library = app.library
    val signedIn = app.account.signedIn
    val online by app.network.online.collectAsState()
    var chip by remember { mutableStateOf<dev.yarek.core.HomeChip?>(null) }
    var chips by remember { mutableStateOf<List<dev.yarek.core.HomeChip>>(emptyList()) }
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
        // The colours scroll away with the top of the list, as in YouTube Music.
        Box(Modifier.graphicsLayer { translationY = if (list.firstVisibleItemIndex == 0) -list.firstVisibleItemScrollOffset.toFloat() else -10000f }) {
            AmbientBackdrop(shelves.firstOrNull()?.items?.firstOrNull()?.let { it.track?.thumbnailUrl ?: it.thumbnail })
        }
        PullToRefreshBox(home.loading && home.value != null, onRefresh = home::retry, Modifier.fillMaxSize()) {
            val history = library.history.take(20).map { it.toItem() }
            LazyColumn(state = list, contentPadding = PaddingValues(bottom = 24.dp), verticalArrangement = Arrangement.spacedBy(24.dp)) {
                item { HomeHeader() }
                // The mood chips stay at the top while the shelves scroll under them.
                if (chips.isNotEmpty()) {
                    stickyHeader {
                        val stuck = list.firstVisibleItemIndex >= 1
                        LazyRow(
                            Modifier.fillMaxWidth().background(if (stuck) YT.background else Color.Transparent)
                                .then(if (stuck) Modifier.statusBarsPadding() else Modifier).padding(vertical = 8.dp),
                            contentPadding = PaddingValues(horizontal = 16.dp), horizontalArrangement = Arrangement.spacedBy(8.dp),
                        ) {
                            items(chips, key = { it.params }) { c ->
                                ChipButton(c.title, chip == c) { chip = if (chip == c) null else c }
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

fun Track.toItem() = dev.yarek.core.Item(ItemKind.Song, videoId, title, artist, track = this, explicit = explicit)

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

    val focus = remember { androidx.compose.ui.focus.FocusRequester() }
    LaunchedEffect(Unit) { if (searched.isEmpty()) runCatching { focus.requestFocus() } }
    Column(Modifier.fillMaxSize().statusBarsPadding()) {
        // YouTube Music's search bar: ←, a dark rounded field.
        Row(Modifier.fillMaxWidth().padding(end = 16.dp, top = 4.dp, bottom = 4.dp), verticalAlignment = Alignment.CenterVertically) {
        IconButton(onClick = { Router.back() }) { Icon(Icons.AutoMirrored.Filled.ArrowBack, stringResource(R.string.back)) }
        Row(
            Modifier.weight(1f).height(44.dp)
                .clip(RoundedCornerShape(22.dp)).background(YT.surface).padding(start = 18.dp, end = 4.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Box(Modifier.weight(1f)) {
                if (query.isEmpty()) Text(stringResource(R.string.search_prompt), color = YT.secondary, fontSize = 16.sp)
                androidx.compose.foundation.text.BasicTextField(
                    query, { query = it }, Modifier.fillMaxWidth().focusRequester(focus), singleLine = true,
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
                }) { Icon(Icons.Default.Clear, stringResource(R.string.clear)) }
            }
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
            searched.isEmpty() -> RecentSearches { run(it) }
            else -> Results(searched, kind) { kind = it }
        }
    }
}

/** The Explore tab: the header, then YouTube Music's Explore shelves (new releases, moods…). */
@Composable
fun ExploreScreen() {
    val explore = rememberLoad(Unit) { App.instance.catalog.explore() }
    Column(Modifier.fillMaxSize()) {
        HomeHeader()
        Loaded(explore) { shelves ->
            LazyColumn(contentPadding = PaddingValues(top = 16.dp, bottom = 16.dp), verticalArrangement = Arrangement.spacedBy(28.dp)) {
                items(shelves, key = { it.id }) { ShelfView(it) }
            }
        }
    }
}

/** Before anything is typed: the songs played lately, to search for again. */
@Composable
private fun RecentSearches(onPick: (String) -> Unit) {
    val history = App.instance.library.history.take(12)
    if (history.isEmpty()) return
    LazyColumn {
        item { Text(stringResource(R.string.recently_played), Modifier.padding(16.dp), fontSize = 16.sp, fontWeight = FontWeight.Bold) }
        itemsIndexed(history) { i, t -> TrackRow(t, history, i) }
    }
}

@Composable
private fun Results(query: String, kind: SearchKind, onKind: (SearchKind) -> Unit) {
    val results = rememberLoad(query to kind) { App.instance.catalog.search(query, kind) }
    Column {
        LazyRow(Modifier.padding(vertical = 8.dp), contentPadding = PaddingValues(horizontal = 16.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
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
    // An artist's picture fills the top, under the bar; other pages start below it.
    val artist = (loaded?.kind ?: preview?.kind) == ItemKind.Artist
    Box(Modifier.fillMaxSize()) {
        if (!artist) AmbientBackdrop(loaded?.thumbnail ?: preview?.thumbnail, 520.dp)
        Column(Modifier.fillMaxSize()) {
            if (!artist) TopBar("", back = true)
            when {
                loaded != null -> PageContent(loaded, preview?.title.orEmpty(), showAll) { showAll = true }
                // The card's title and art at once, the songs when they come.
                preview != null -> PagePreview(preview, page)
                else -> Loaded(page) {}
            }
        }
        if (artist) TopBar("", back = true)
    }
}

@Composable
private fun PagePreview(item: dev.yarek.core.Item, page: Load<Page>) {
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
                Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp), verticalAlignment = Alignment.CenterVertically) {
                    Text(stringResource(R.string.top_songs), Modifier.weight(1f), fontSize = 24.sp, fontWeight = FontWeight.Bold)
                    OutlinePill(stringResource(R.string.play_all)) { playFrom(page.tracks, 0) }
                }
            }
        }
        itemsIndexed(shown) { i, t ->
            TrackRow(t, page.tracks, i, number = if (page.kind == ItemKind.Album) i + 1 else null, art = if (page.kind == ItemKind.Artist) 56.dp else 48.dp)
        }
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

/**
 * Art, title, subtitle, then YouTube Music's circles: download, save, play,
 * share and ⋮ (also before the songs are in). Artists get their own header.
 */
@Composable
private fun PageHeader(title: String, subtitle: String, thumbnail: String?, round: Boolean, tracks: List<Track>, page: Page?) {
    if (round) return ArtistHeader(title, subtitle, thumbnail, tracks, page)
    val library = App.instance.library
    val context = LocalContext.current
    Column(Modifier.fillMaxWidth().padding(horizontal = 16.dp), horizontalAlignment = Alignment.CenterHorizontally) {
        Artwork(thumbnail?.let { Art.Url(it) } ?: Art.None, 220.dp)
        Spacer(Modifier.height(20.dp))
        Text(title, fontSize = 28.sp, fontWeight = FontWeight.Bold, textAlign = TextAlign.Center, lineHeight = 34.sp, maxLines = 2, overflow = TextOverflow.Ellipsis)
        if (subtitle.isNotEmpty()) {
            Spacer(Modifier.height(6.dp))
            Text(subtitle, fontSize = 14.sp, color = YT.secondary, textAlign = TextAlign.Center)
        }
        Spacer(Modifier.height(20.dp))
        Row(horizontalArrangement = Arrangement.spacedBy(16.dp), verticalAlignment = Alignment.CenterVertically) {
            DownloadAllButton(tracks)
            val saved = page != null && library.isSaved(page.id)
            CircleIcon(
                if (saved) Icons.Default.Bookmark else Icons.Default.BookmarkBorder,
                stringResource(if (saved) R.string.remove_from_library else R.string.save), enabled = page != null,
            ) { page?.let(library::toggleSaved) }
            CircleIcon(Icons.Default.PlayArrow, stringResource(R.string.play), 64.dp, filled = true, enabled = tracks.isNotEmpty()) {
                playFrom(tracks, 0)
            }
            CircleIcon(Icons.Default.Share, stringResource(R.string.share), enabled = page != null) {
                page?.let { share(context, "https://music.youtube.com/browse/${it.id}") }
            }
            Box {
                var menu by remember { mutableStateOf(false) }
                CircleIcon(Icons.Default.MoreVert, stringResource(R.string.more), enabled = tracks.isNotEmpty()) { menu = true }
                DropdownMenu(menu, { menu = false }) {
                    DropdownMenuItem(
                        text = { Text(stringResource(R.string.shuffle)) },
                        leadingIcon = { Icon(Icons.Default.Shuffle, null) },
                        onClick = {
                            menu = false
                            playFrom(tracks.shuffled(), 0)
                        },
                    )
                    DropdownMenuItem(
                        text = { Text(stringResource(R.string.start_mix)) },
                        leadingIcon = { Icon(Icons.Default.Sensors, null) },
                        onClick = {
                            menu = false
                            tracks.firstOrNull()?.let(App.instance.player::startRadio)
                        },
                    )
                }
            }
        }
        Spacer(Modifier.height(16.dp))
    }
}

/**
 * An artist: the picture across the whole width with the name over its
 * bottom, then Subscribe, radio and play.
 */
@Composable
private fun ArtistHeader(title: String, subtitle: String, thumbnail: String?, tracks: List<Track>, page: Page?) {
    val library = App.instance.library
    Column(Modifier.fillMaxWidth()) {
        Box(Modifier.fillMaxWidth().aspectRatio(1.05f)) {
            coil.compose.AsyncImage(
                thumbnail?.let { heroUrl(it, 1080, 1028) }, null, Modifier.fillMaxSize().background(YT.surface),
                contentScale = androidx.compose.ui.layout.ContentScale.Crop,
            )
            Box(Modifier.fillMaxSize().background(Brush.verticalGradient(0f to Color.Black.copy(alpha = 0.35f), 0.25f to Color.Transparent, 0.6f to Color.Transparent, 1f to YT.background)))
            Column(Modifier.align(Alignment.BottomStart).padding(horizontal = 16.dp)) {
                Text(title, fontSize = 40.sp, fontWeight = FontWeight.Bold, lineHeight = 44.sp, maxLines = 2, overflow = TextOverflow.Ellipsis)
                if (subtitle.isNotEmpty()) Text(subtitle, fontSize = 14.sp, color = YT.secondary, maxLines = 1, overflow = TextOverflow.Ellipsis)
            }
        }
        Row(Modifier.fillMaxWidth().padding(16.dp), verticalAlignment = Alignment.CenterVertically) {
            val saved = page != null && library.isSaved(page.id)
            Box(
                Modifier.height(40.dp).clip(CircleShape)
                    .background(if (saved) Color.White.copy(alpha = 0.1f) else Color.White)
                    .clickable(enabled = page != null) { page?.let(library::toggleSaved) }
                    .padding(horizontal = 18.dp),
                contentAlignment = Alignment.Center,
            ) {
                Text(
                    stringResource(if (saved) R.string.subscribed else R.string.subscribe),
                    color = if (saved) Color.White else Color.Black, fontWeight = FontWeight.Medium, fontSize = 15.sp,
                )
            }
            Spacer(Modifier.weight(1f))
            CircleIcon(Icons.Default.Sensors, stringResource(R.string.start_mix), 40.dp, enabled = tracks.isNotEmpty()) {
                tracks.firstOrNull()?.let(App.instance.player::startRadio)
            }
            Spacer(Modifier.width(12.dp))
            CircleIcon(Icons.Default.PlayArrow, stringResource(R.string.play), 56.dp, filled = true, enabled = tracks.isNotEmpty()) {
                playFrom(tracks, 0)
            }
        }
    }
}

fun share(context: android.content.Context, url: String) {
    val send = android.content.Intent(android.content.Intent.ACTION_SEND)
        .setType("text/plain")
        .putExtra(android.content.Intent.EXTRA_TEXT, url)
    context.startActivity(android.content.Intent.createChooser(send, null))
}

/** Play and shuffle for a list (liked songs, a playlist…). */
@Composable
fun PlayButtons(tracks: List<Track>) {
    Row(horizontalArrangement = Arrangement.spacedBy(28.dp), verticalAlignment = Alignment.CenterVertically) {
        CircleIcon(Icons.Default.PlayArrow, stringResource(R.string.play), 64.dp, filled = true, enabled = tracks.isNotEmpty()) {
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

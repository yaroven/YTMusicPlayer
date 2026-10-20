package dev.ytmplayer.android.ui

import androidx.compose.material.icons.filled.ChevronRight
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.gestures.snapping.rememberSnapFlingBehavior
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.LazyHorizontalGrid
import androidx.compose.foundation.lazy.grid.items
import androidx.compose.foundation.lazy.grid.itemsIndexed
import androidx.compose.foundation.lazy.grid.rememberLazyGridState
import androidx.compose.material.icons.filled.GraphicEq
import androidx.compose.material.icons.filled.PlayArrow
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.text.style.TextAlign
import dev.ytmplayer.core.ShelfStyle
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.requiredSize
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.PlaylistAdd
import androidx.compose.material.icons.automirrored.filled.QueueMusic
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.ArrowDownward
import androidx.compose.material.icons.filled.ArrowUpward
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.DownloadForOffline
import androidx.compose.material.icons.filled.Download
import androidx.compose.material.icons.filled.Favorite
import androidx.compose.material.icons.filled.FavoriteBorder
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material.icons.filled.MusicNote
import androidx.compose.material.icons.filled.Person
import androidx.compose.material.icons.filled.Radio
import androidx.compose.material.icons.filled.ThumbDown
import androidx.compose.material.icons.automirrored.filled.PlaylistPlay
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import coil.compose.AsyncImage
import dev.ytmplayer.android.App
import dev.ytmplayer.android.R
import dev.ytmplayer.android.formatTime
import dev.ytmplayer.core.Item
import dev.ytmplayer.core.ItemKind
import dev.ytmplayer.core.Shelf
import dev.ytmplayer.core.Track

/** What a picture shows: a video's thumbnail, a URL, or nothing. */
sealed interface Art {
    data class Video(val id: String) : Art
    data class Url(val url: String) : Art
    object None : Art
}

val Item.art: Art get() = track?.let { Art.Video(it.videoId) } ?: thumbnail?.let { Art.Url(it) } ?: Art.None

/** googleusercontent thumbnails take their size in the URL (`=w226-h226…`). */
private fun resized(url: String, px: Int): String {
    if (!url.contains("googleusercontent.com")) return url
    val cut = url.lastIndexOf("=w").takeIf { it >= 0 } ?: return url
    return url.substring(0, cut) + "=w$px-h$px-l90-rj"
}

/**
 * Square cover art. Video thumbnails are 16:9 (`mqdefault`,
 * `maxresdefault`) or 16:9 inside a letterboxed 4:3 picture (`hqdefault`);
 * both show the centre square. Big covers ask for `maxresdefault` and fall
 * back to `hqdefault` where YouTube has none.
 */
@Composable
fun Artwork(art: Art, size: Dp, modifier: Modifier = Modifier, round: Boolean = false) {
    val first = when (art) {
        is Art.Video -> "https://i.ytimg.com/vi/${art.id}/" +
            (if (size > 240.dp) "maxresdefault" else if (size > 120.dp) "hqdefault" else "mqdefault") + ".jpg"
        is Art.Url -> resized(art.url, (size.value * 3).toInt())
        Art.None -> null
    }
    var url by remember(first) { mutableStateOf(first) }
    val letterboxed = url?.let { it.contains("i.ytimg.com") && (it.contains("hqdefault") || it.contains("sddefault")) } == true
    Box(
        modifier
            .size(size)
            .clip(if (round) CircleShape else RoundedCornerShape(if (size > 100.dp) 10.dp else 5.dp))
            .background(Color.White.copy(alpha = 0.08f)),
        contentAlignment = Alignment.Center,
    ) {
        if (url == null) {
            Icon(Icons.Default.MusicNote, null, tint = MaterialTheme.colorScheme.onSurfaceVariant)
        } else {
            AsyncImage(
                model = url,
                contentDescription = null,
                contentScale = ContentScale.Crop,
                modifier = if (letterboxed) Modifier.requiredSize(size * 16 / 9, size * 4 / 3) else Modifier.size(size),
                onError = {
                    if (art is Art.Video && url?.contains("maxresdefault") == true) {
                        url = "https://i.ytimg.com/vi/${art.id}/hqdefault.jpg"
                    }
                },
            )
        }
    }
}

/** YouTube Music's "E": explicit lyrics (18+). */
@Composable
fun ExplicitBadge() {
    Box(
        Modifier.size(14.dp).background(YT.secondary, RoundedCornerShape(2.dp)),
        contentAlignment = Alignment.Center,
    ) {
        Text("E", fontSize = 10.sp, fontWeight = FontWeight.Bold, color = Color.Black)
    }
}

/** An extra entry in a song's menu (playlist moves, removal…). */
class MenuAction(val label: String, val icon: androidx.compose.ui.graphics.vector.ImageVector, val run: () -> Unit)

/**
 * Plays `list` from `index`. Offline, only downloaded songs can play: the
 * downloaded part of the list, from this song.
 */
fun playFrom(list: List<Track>, index: Int) {
    val app = App.instance
    val track = list.getOrNull(index) ?: return
    if (!app.network.online.value && !app.library.isDownloaded(track.videoId)) {
        Notice.show(app.getString(R.string.offline_not_downloaded))
        return
    }
    if (!app.network.online.value) {
        val offline = list.filter { app.library.isDownloaded(it.videoId) }
        app.player.play(offline, offline.indexOfFirst { it.videoId == track.videoId }.coerceAtLeast(0))
        return
    }
    app.player.play(list, index)
}

/** A song in a list: art (or its number on an album), title, artist • length, ⋮. */
@OptIn(ExperimentalFoundationApi::class)
@Composable
fun TrackRow(
    track: Track,
    list: List<Track>,
    index: Int,
    number: Int? = null,
    extra: List<MenuAction> = emptyList(),
    /** Shown when the list is sorted by plays. */
    plays: Int? = null,
) {
    val app = App.instance
    val player = app.player
    val library = app.library
    val online by app.network.online.collectAsState()
    var menu by remember { mutableStateOf(false) }
    val current = player.current?.videoId == track.videoId
    val downloaded = library.isDownloaded(track.videoId)
    Row(
        Modifier
            .fillMaxWidth()
            .combinedClickable(onClick = { playFrom(list, index) }, onLongClick = { menu = true })
            // Offline, what can't play is dimmed.
            .alpha(if (!online && !downloaded) 0.4f else 1f)
            .padding(start = 16.dp, end = 4.dp, top = 6.dp, bottom = 6.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        if (number != null) {
            Text(
                "$number", Modifier.width(28.dp), fontSize = 15.sp,
                color = if (current) Color.White else YT.secondary,
            )
        } else {
            Box {
                Artwork(Art.Video(track.videoId), 56.dp)
                if (current) {
                    Box(
                        Modifier.size(56.dp).clip(RoundedCornerShape(5.dp)).background(Color.Black.copy(alpha = 0.5f)),
                        contentAlignment = Alignment.Center,
                    ) {
                        Icon(
                            if (player.isPlaying) Icons.Default.GraphicEq else Icons.Default.PlayArrow, null,
                            tint = Color.White,
                        )
                    }
                }
            }
        }
        Spacer(Modifier.width(16.dp))
        Column(Modifier.weight(1f)) {
            Text(track.title, maxLines = 1, overflow = TextOverflow.Ellipsis, fontSize = 17.sp)
            Row(verticalAlignment = Alignment.CenterVertically) {
                if (track.explicit) {
                    ExplicitBadge()
                    Spacer(Modifier.width(5.dp))
                }
                if (downloaded) {
                    Icon(Icons.Default.DownloadForOffline, stringResource(R.string.downloaded), Modifier.size(13.dp), tint = YT.secondary)
                    Spacer(Modifier.width(5.dp))
                }
                val duration = track.durationSecs?.let { " • " + formatTime(it.toDouble()) }.orEmpty()
                val count = plays?.let { " • ▶ $it" }.orEmpty()
                Text(track.artist + duration + count, maxLines = 1, overflow = TextOverflow.Ellipsis, fontSize = 14.sp, color = YT.secondary)
            }
        }
        Box {
            IconButton(onClick = { menu = true }) { Icon(Icons.Default.MoreVert, stringResource(R.string.more), tint = YT.secondary) }
            TrackMenu(track, menu, { menu = false }, extra)
        }
    }
}

/** A song's actions: queue, radio, like, playlists, download, artist. */
@Composable
fun TrackMenu(track: Track, open: Boolean, onDismiss: () -> Unit, extra: List<MenuAction> = emptyList()) {
    val app = App.instance
    val player = app.player
    val library = app.library
    var choosing by remember { mutableStateOf(false) }
    DropdownMenu(open, onDismiss) {
        @Composable
        fun entry(label: String, icon: androidx.compose.ui.graphics.vector.ImageVector, enabled: Boolean = true, run: () -> Unit) =
            DropdownMenuItem(
                text = { Text(label) }, leadingIcon = { Icon(icon, null) }, enabled = enabled,
                onClick = {
                    onDismiss()
                    run()
                },
            )
        entry(stringResource(R.string.play_next), Icons.AutoMirrored.Filled.PlaylistPlay) { player.playNext(track) }
        entry(stringResource(R.string.add_to_queue), Icons.AutoMirrored.Filled.QueueMusic) { player.addToQueue(track) }
        entry(stringResource(R.string.start_radio), Icons.Default.Radio) { player.startRadio(track) }
        HorizontalDivider()
        val liked = library.isLiked(track)
        entry(
            stringResource(if (liked) R.string.remove_from_liked else R.string.like),
            if (liked) Icons.Default.Favorite else Icons.Default.FavoriteBorder,
        ) { library.toggleLike(track) }
        entry(stringResource(R.string.add_to_playlist), Icons.AutoMirrored.Filled.PlaylistAdd) { choosing = true }
        when {
            library.isDownloaded(track.videoId) ->
                entry(stringResource(R.string.remove_download), Icons.Default.Delete) { library.removeDownload(track) }
            track.videoId in library.downloading ->
                entry(stringResource(R.string.downloading), Icons.Default.Download, enabled = false) {}
            else -> entry(stringResource(R.string.download), Icons.Default.Download) { library.download(track) }
        }
        entry(stringResource(R.string.dislike), Icons.Default.ThumbDown) { player.dislike(track) }
        entry(stringResource(R.string.go_to_artist), Icons.Default.Person) { Router.openArtist(track) }
        if (extra.isNotEmpty()) HorizontalDivider()
        extra.forEach { entry(it.label, it.icon, run = it.run) }
    }
    if (choosing) PlaylistChooser(track) { choosing = false }
}

/** "Add to playlist": the playlists, and a new one. */
@Composable
fun PlaylistChooser(track: Track, onDismiss: () -> Unit) {
    val library = App.instance.library
    var naming by remember { mutableStateOf(false) }
    if (naming) {
        NameDialog(stringResource(R.string.new_playlist), "", onDismiss) {
            library.createPlaylist(it, listOf(track))
            onDismiss()
        }
        return
    }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(stringResource(R.string.add_to_playlist)) },
        text = {
            Column {
                library.playlists.forEach { p ->
                    Text(
                        p.name,
                        Modifier.fillMaxWidth().clickable {
                            library.add(track, p.id)
                            onDismiss()
                        }.padding(vertical = 12.dp),
                    )
                }
                Row(
                    Modifier.fillMaxWidth().clickable { naming = true }.padding(vertical = 12.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Icon(Icons.Default.Add, null)
                    Spacer(Modifier.width(8.dp))
                    Text(stringResource(R.string.new_playlist))
                }
            }
        },
        confirmButton = {},
        dismissButton = { TextButton(onClick = onDismiss) { Text(stringResource(R.string.cancel)) } },
    )
}

@Composable
fun NameDialog(title: String, initial: String, onDismiss: () -> Unit, onDone: (String) -> Unit) {
    var name by remember { mutableStateOf(initial) }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(title) },
        text = {
            OutlinedTextField(name, { name = it }, singleLine = true, label = { Text(stringResource(R.string.name)) })
        },
        confirmButton = { TextButton(onClick = { onDone(name) }) { Text(stringResource(R.string.ok)) } },
        dismissButton = { TextButton(onClick = onDismiss) { Text(stringResource(R.string.cancel)) } },
    )
}

/** An album, artist or playlist in a list. */
@Composable
fun ItemRow(item: Item) {
    LaunchedEffect(item.browseId) { PageCache.note(item) }
    Row(
        Modifier.fillMaxWidth().prefetchOnPress(item).clickable { Router.open(Route.Page(item.browseId)) }
            .padding(horizontal = 16.dp, vertical = 6.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Artwork(item.art, 56.dp, round = item.kind == ItemKind.Artist)
        Spacer(Modifier.width(14.dp))
        Column(Modifier.weight(1f)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(item.title, maxLines = 1, overflow = TextOverflow.Ellipsis, fontSize = 16.sp, modifier = Modifier.weight(1f, fill = false))
                if (item.explicit) {
                    Spacer(Modifier.width(6.dp))
                    ExplicitBadge()
                }
            }
            Text(item.subtitle, maxLines = 1, overflow = TextOverflow.Ellipsis, fontSize = 14.sp, color = YT.secondary)
        }
    }
}

/** An Explore category ("Chill", "Workout"): a dark tile with a coloured edge, as YouTube Music draws them. */
@Composable
private fun MoodTile(item: Item) {
    val hue = (item.title.hashCode() and 0xFFFF) % 360
    LaunchedEffect(item.browseId) { PageCache.note(item) }
    Row(
        Modifier
            .width(170.dp)
            .height(48.dp)
            .clip(RoundedCornerShape(6.dp))
            .background(YT.surface)
            .prefetchOnPress(item)
            .clickable { Router.open(Route.Page(item.browseId)) },
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(Modifier.width(6.dp).fillMaxHeight().background(Color.hsv(hue.toFloat(), 0.7f, 0.9f)))
        Text(
            item.title, Modifier.padding(horizontal = 12.dp), fontWeight = FontWeight.SemiBold, fontSize = 15.sp,
            maxLines = 1, overflow = TextOverflow.Ellipsis,
        )
    }
}

/** A card in a shelf: square art (round for artists, 16:9 for videos), title and subtitle under it. */
@OptIn(ExperimentalFoundationApi::class)
@Composable
fun Card(item: Item, songs: List<Track>, wide: Boolean = false) {
    var menu by remember { mutableStateOf(false) }
    val track = item.track
    val width = if (wide) 280.dp else 160.dp
    val round = item.kind == ItemKind.Artist
    LaunchedEffect(item.browseId) { PageCache.note(item) }
    Column(
        Modifier
            .width(width)
            .prefetchOnPress(item)
            .combinedClickable(
                onClick = {
                    if (track != null) {
                        val list = songs.ifEmpty { listOf(track) }
                        playFrom(list, list.indexOf(track).coerceAtLeast(0))
                    } else {
                        Router.open(Route.Page(item.browseId))
                    }
                },
                onLongClick = { if (track != null) menu = true },
            ),
        horizontalAlignment = if (round) Alignment.CenterHorizontally else Alignment.Start,
    ) {
        Box {
            if (wide) {
                coil.compose.AsyncImage(
                    model = track?.let { "https://i.ytimg.com/vi/${it.videoId}/mqdefault.jpg" } ?: item.thumbnail,
                    contentDescription = null,
                    contentScale = ContentScale.Crop,
                    modifier = Modifier.width(width).aspectRatio(16f / 9f).clip(RoundedCornerShape(6.dp)).background(YT.raised),
                )
            } else {
                Artwork(item.art, width, round = round)
            }
            if (item.explicit) Box(Modifier.align(Alignment.BottomStart).padding(6.dp)) { ExplicitBadge() }
            if (track != null) TrackMenu(track, menu, { menu = false })
        }
        Spacer(Modifier.height(8.dp))
        Text(
            item.title, fontSize = 15.sp, fontWeight = FontWeight.Medium, maxLines = 2, overflow = TextOverflow.Ellipsis,
            textAlign = if (round) TextAlign.Center else TextAlign.Start,
        )
        Text(
            item.subtitle, fontSize = 13.sp, color = YT.secondary, maxLines = 2, overflow = TextOverflow.Ellipsis,
            textAlign = if (round) TextAlign.Center else TextAlign.Start,
        )
    }
}

/**
 * A shelf: its header (strapline, title, "Play all"), then its cards — in a
 * row, as a three-row grid of tiles (song shelves like "Quick picks"), as
 * wide video cards, or as mood tiles.
 */
@Composable
fun ShelfView(shelf: Shelf, showTitle: Boolean = true) {
    val songs = shelf.items.mapNotNull { it.track }
    val moods = shelf.items.all { it.track == null && it.thumbnail == null && it.subtitle.isEmpty() }
    Column {
        if (showTitle && shelf.title.isNotEmpty()) {
            Row(
                Modifier.fillMaxWidth().padding(horizontal = 16.dp).padding(bottom = 12.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                shelf.straplineArt?.let {
                    Artwork(Art.Url(it), 36.dp, round = true)
                    Spacer(Modifier.width(12.dp))
                }
                Column(Modifier.weight(1f)) {
                    if (shelf.strapline.isNotEmpty()) {
                        Text(shelf.strapline.uppercase(), fontSize = 12.sp, fontWeight = FontWeight.Medium, color = YT.secondary)
                    }
                    Text(shelf.title, fontSize = 22.sp, fontWeight = FontWeight.Bold, maxLines = 1, overflow = TextOverflow.Ellipsis)
                }
                // Videos keep "Play all"; song shelves show YouTube Music's ›, which plays them.
                if (songs.size > 1) {
                    if (shelf.style == ShelfStyle.Wide) {
                        OutlinePill(stringResource(R.string.play_all)) { playFrom(songs, 0) }
                    } else {
                        Box(
                            Modifier.size(36.dp).clip(CircleShape).border(1.dp, Color.White.copy(alpha = 0.3f), CircleShape)
                                .clickable { playFrom(songs, 0) },
                            contentAlignment = Alignment.Center,
                        ) { Icon(Icons.Default.ChevronRight, stringResource(R.string.play_all), Modifier.size(22.dp)) }
                    }
                }
            }
        }
        when {
            moods -> LazyHorizontalGrid(
                rows = GridCells.Fixed(2),
                modifier = Modifier.height(106.dp),
                contentPadding = PaddingValues(horizontal = 16.dp),
                horizontalArrangement = Arrangement.spacedBy(10.dp),
                verticalArrangement = Arrangement.spacedBy(10.dp),
            ) { items(shelf.items, key = { it.id }) { MoodTile(it) } }
            shelf.style == ShelfStyle.Grid && songs.isNotEmpty() -> QuickPicks(songs)
            else -> LazyRow(contentPadding = PaddingValues(horizontal = 16.dp), horizontalArrangement = Arrangement.spacedBy(14.dp)) {
                items(shelf.items, key = { it.id }) { Card(it, songs, wide = shelf.style == ShelfStyle.Wide) }
            }
        }
    }
}

/** "Quick picks": square tiles three rows high with the title over the picture, paged sideways, dots under them. */
@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun QuickPicks(songs: List<Track>) {
    val player = App.instance.player
    BoxWithConstraints(Modifier.fillMaxWidth()) {
        val tile = (maxWidth - 32.dp - 12.dp) / 3
        val state = rememberLazyGridState()
        val pages = (songs.size + 8) / 9
        Column {
            LazyHorizontalGrid(
                rows = GridCells.Fixed(3),
                state = state,
                flingBehavior = rememberSnapFlingBehavior(state),
                modifier = Modifier.height(tile * 3 + 12.dp),
                contentPadding = PaddingValues(horizontal = 16.dp),
                horizontalArrangement = Arrangement.spacedBy(6.dp),
                verticalArrangement = Arrangement.spacedBy(6.dp),
            ) {
                itemsIndexed(songs, key = { _, t -> t.videoId }) { i, track ->
                    var menu by remember { mutableStateOf(false) }
                    Box(
                        Modifier
                            .size(tile)
                            .clip(RoundedCornerShape(5.dp))
                            .combinedClickable(onClick = { playFrom(songs, i) }, onLongClick = { menu = true })
                            .then(
                                if (player.current?.videoId == track.videoId) {
                                    Modifier.border(3.dp, Color.White, RoundedCornerShape(5.dp))
                                } else {
                                    Modifier
                                },
                            ),
                        contentAlignment = Alignment.BottomStart,
                    ) {
                        Artwork(Art.Video(track.videoId), tile)
                        Box(
                            Modifier.matchParentSize().background(
                                Brush.verticalGradient(0.5f to Color.Transparent, 1f to Color.Black.copy(alpha = 0.75f)),
                            ),
                        )
                        Text(
                            track.title, Modifier.padding(horizontal = 8.dp, vertical = 7.dp),
                            fontSize = 14.sp, fontWeight = FontWeight.SemiBold, maxLines = 1, overflow = TextOverflow.Ellipsis,
                        )
                        TrackMenu(track, menu, { menu = false })
                    }
                }
            }
            if (pages > 1) {
                val page = ((state.firstVisibleItemIndex + 4) / 9).coerceIn(0, pages - 1)
                Row(Modifier.fillMaxWidth().padding(top = 12.dp), horizontalArrangement = Arrangement.Center) {
                    repeat(pages) { p ->
                        Box(
                            Modifier.padding(horizontal = 3.dp).size(7.dp).clip(CircleShape)
                                .background(Color.White.copy(alpha = if (p == page) 1f else 0.35f)),
                        )
                    }
                }
            }
        }
    }
}

fun moveActions(up: (() -> Unit)?, down: (() -> Unit)?, upLabel: String, downLabel: String): List<MenuAction> =
    listOfNotNull(
        up?.let { MenuAction(upLabel, Icons.Default.ArrowUpward, it) },
        down?.let { MenuAction(downLabel, Icons.Default.ArrowDownward, it) },
    )

package dev.ytmplayer.android.ui

import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.material.icons.filled.DownloadDone
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.Share
import androidx.compose.material.icons.filled.Sensors
import androidx.compose.material.icons.outlined.Person
import androidx.compose.material.icons.outlined.ThumbDown
import androidx.compose.material.icons.outlined.ThumbUp
import androidx.compose.material.icons.filled.ThumbUp
import androidx.compose.ui.platform.LocalContext
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.ExperimentalMaterial3Api
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

/** An artist's picture big enough for the whole width, cropped to `w`×`h` by Google. */
fun heroUrl(url: String, w: Int, h: Int): String {
    if (!url.contains("googleusercontent.com")) return url
    val cut = url.lastIndexOf("=w").takeIf { it >= 0 } ?: return url
    return url.substring(0, cut) + "=w$w-h$h-p-l90-rj"
}

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
    art: Dp = 48.dp,
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
            .height(art + 16.dp)
            .padding(start = 16.dp, end = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        if (number != null) {
            Text(
                "$number", Modifier.width(40.dp), fontSize = 16.sp, textAlign = TextAlign.Center,
                color = if (current) Color.White else YT.secondary,
            )
        } else {
            Box {
                Artwork(Art.Video(track.videoId), art)
                if (current) {
                    Box(
                        Modifier.size(art).clip(RoundedCornerShape(5.dp)).background(Color.Black.copy(alpha = 0.5f)),
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
            Text(track.title, maxLines = 1, overflow = TextOverflow.Ellipsis, fontSize = 16.sp)
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

/**
 * A song's actions in YouTube Music's sheet: the song with 👍 and ✕, three
 * tiles (play next, save to playlist, share), then the rest as a list.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun TrackMenu(track: Track, open: Boolean, onDismiss: () -> Unit, extra: List<MenuAction> = emptyList()) {
    val app = App.instance
    val player = app.player
    val library = app.library
    val context = LocalContext.current
    var choosing by remember { mutableStateOf(false) }
    if (choosing) PlaylistChooser(track) { choosing = false }
    if (!open) return
    ModalBottomSheet(
        onDismissRequest = onDismiss,
        sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
        modifier = Modifier.padding(horizontal = 8.dp),
        shape = RoundedCornerShape(12.dp),
        containerColor = YT.surface, contentColor = Color.White,
        dragHandle = null,
    ) {
        fun run(action: () -> Unit) {
            onDismiss()
            action()
        }
        Column(Modifier.fillMaxWidth().verticalScroll(rememberScrollState()).navigationBarsPadding().padding(bottom = 8.dp)) {
            Row(Modifier.fillMaxWidth().padding(start = 16.dp, top = 12.dp, bottom = 12.dp, end = 4.dp), verticalAlignment = Alignment.CenterVertically) {
                Column(Modifier.weight(1f)) {
                    Text(track.title, fontSize = 18.sp, fontWeight = FontWeight.Bold, maxLines = 1, overflow = TextOverflow.Ellipsis)
                    val duration = track.durationSecs?.let { " • " + formatTime(it.toDouble()) }.orEmpty()
                    Text(track.artist + duration, fontSize = 14.sp, color = YT.secondary, maxLines = 1, overflow = TextOverflow.Ellipsis)
                }
                val liked = library.isLiked(track)
                IconButton(onClick = { library.toggleLike(track) }) {
                    Icon(if (liked) Icons.Filled.ThumbUp else Icons.Outlined.ThumbUp, stringResource(R.string.like))
                }
                IconButton(onClick = onDismiss) { Icon(Icons.Default.Close, stringResource(R.string.close)) }
            }
            Box(Modifier.fillMaxWidth().height(1.dp).background(Color.White.copy(alpha = 0.1f)))
            Row(Modifier.fillMaxWidth().padding(16.dp), horizontalArrangement = Arrangement.spacedBy(16.dp)) {
                @Composable
                fun tile(icon: androidx.compose.ui.graphics.vector.ImageVector, label: String, action: () -> Unit) {
                    Column(Modifier.weight(1f).clickable { run(action) }, horizontalAlignment = Alignment.CenterHorizontally) {
                        Box(
                            Modifier.fillMaxWidth().height(64.dp).clip(RoundedCornerShape(8.dp)).background(Color.White.copy(alpha = 0.1f)),
                            contentAlignment = Alignment.Center,
                        ) { Icon(icon, null) }
                        Spacer(Modifier.height(8.dp))
                        Text(label, fontSize = 13.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
                    }
                }
                tile(Icons.AutoMirrored.Filled.PlaylistPlay, stringResource(R.string.play_next)) { player.playNext(track) }
                tile(Icons.AutoMirrored.Filled.PlaylistAdd, stringResource(R.string.save_to_playlist)) { choosing = true }
                tile(Icons.Default.Share, stringResource(R.string.share)) {
                    val send = android.content.Intent(android.content.Intent.ACTION_SEND)
                        .setType("text/plain")
                        .putExtra(android.content.Intent.EXTRA_TEXT, "https://music.youtube.com/watch?v=${track.videoId}")
                    context.startActivity(android.content.Intent.createChooser(send, null))
                }
            }
            @Composable
            fun entry(label: String, icon: androidx.compose.ui.graphics.vector.ImageVector, enabled: Boolean = true, action: () -> Unit) {
                Row(
                    Modifier.fillMaxWidth().height(52.dp).clickable(enabled = enabled) { run(action) }.padding(horizontal = 24.dp)
                        .alpha(if (enabled) 1f else 0.4f),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Icon(icon, null)
                    Spacer(Modifier.width(24.dp))
                    Text(label, fontSize = 16.sp)
                }
            }
            entry(stringResource(R.string.start_mix), Icons.Default.Sensors) { player.startRadio(track) }
            entry(stringResource(R.string.add_to_queue), Icons.AutoMirrored.Filled.QueueMusic) { player.addToQueue(track) }
            when {
                library.isDownloaded(track.videoId) ->
                    entry(stringResource(R.string.remove_download), Icons.Default.DownloadDone) { library.removeDownload(track) }
                track.videoId in library.downloading ->
                    entry(stringResource(R.string.downloading), Icons.Default.Download, enabled = false) {}
                else -> entry(stringResource(R.string.download), Icons.Default.Download) { library.download(track) }
            }
            extra.forEach { entry(it.label, it.icon, action = it.run) }
            entry(stringResource(R.string.go_to_artist), Icons.Outlined.Person) { Router.openArtist(track) }
            entry(stringResource(R.string.dislike), Icons.Outlined.ThumbDown) { player.dislike(track) }
        }
    }
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
            .height(72.dp).padding(horizontal = 16.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Artwork(item.art, 56.dp, round = item.kind == ItemKind.Artist)
        Spacer(Modifier.width(16.dp))
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
    val width = if (wide) 347.dp else 173.dp
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
        Spacer(Modifier.height(10.dp))
        Text(
            item.title, fontSize = 16.sp, fontWeight = FontWeight.Medium, maxLines = 1, overflow = TextOverflow.Ellipsis,
            textAlign = if (round) TextAlign.Center else TextAlign.Start,
        )
        Text(
            item.subtitle, fontSize = 14.sp, color = YT.secondary, maxLines = 2, overflow = TextOverflow.Ellipsis,
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
                Modifier.fillMaxWidth().padding(horizontal = 16.dp).padding(bottom = 16.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                shelf.straplineArt?.let {
                    Artwork(Art.Url(it), 40.dp, round = true)
                    Spacer(Modifier.width(12.dp))
                }
                Column(Modifier.weight(1f)) {
                    if (shelf.strapline.isNotEmpty()) {
                        Text(shelf.strapline.uppercase(), fontSize = 12.sp, fontWeight = FontWeight.Medium, color = YT.secondary)
                    }
                    Text(shelf.title, fontSize = 24.sp, fontWeight = FontWeight.Bold, maxLines = 1, overflow = TextOverflow.Ellipsis)
                }
                if (songs.size > 1) OutlinePill(stringResource(R.string.play_all)) { playFrom(songs, 0) }
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
            else -> LazyRow(contentPadding = PaddingValues(horizontal = 16.dp), horizontalArrangement = Arrangement.spacedBy(16.dp)) {
                items(shelf.items, key = { it.id }) { Card(it, songs, wide = shelf.style == ShelfStyle.Wide) }
            }
        }
    }
}

/**
 * "Quick picks" and other song shelves, as YouTube Music draws them on
 * Android: columns of four rows (art, title, artist, ⋮), paged sideways
 * with the next column peeking in.
 */
@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun QuickPicks(songs: List<Track>) {
    val state = rememberLazyListState()
    val columns = songs.chunked(4)
    BoxWithConstraints(Modifier.fillMaxWidth()) {
        val width = maxWidth - 32.dp
        LazyRow(
            state = state,
            flingBehavior = rememberSnapFlingBehavior(state),
        ) {
            itemsIndexed(columns) { c, column ->
                Column(Modifier.width(width)) {
                    column.forEachIndexed { r, track ->
                        Box(Modifier.height(72.dp), contentAlignment = Alignment.Center) {
                            TrackRow(track, songs, c * 4 + r, art = 56.dp)
                        }
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

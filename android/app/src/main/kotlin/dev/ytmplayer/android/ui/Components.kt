package dev.ytmplayer.android.ui

import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
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
        Modifier.size(15.dp).background(Color.White.copy(alpha = 0.15f), RoundedCornerShape(3.dp)),
        contentAlignment = Alignment.Center,
    ) {
        Text("E", fontSize = 10.sp, fontWeight = FontWeight.Bold, color = MaterialTheme.colorScheme.onSurfaceVariant)
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

@OptIn(ExperimentalFoundationApi::class)
@Composable
fun TrackRow(
    track: Track,
    list: List<Track>,
    index: Int,
    number: Int? = null,
    extra: List<MenuAction> = emptyList(),
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
                "$number", Modifier.width(28.dp), style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        } else {
            Artwork(Art.Video(track.videoId), 48.dp)
        }
        Spacer(Modifier.width(12.dp))
        Column(Modifier.weight(1f)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(
                    track.title, maxLines = 1, overflow = TextOverflow.Ellipsis,
                    color = if (current) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurface,
                    modifier = Modifier.weight(1f, fill = false),
                )
                if (track.explicit) {
                    Spacer(Modifier.width(6.dp))
                    ExplicitBadge()
                }
            }
            Text(
                track.artist, maxLines = 1, overflow = TextOverflow.Ellipsis,
                style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        if (downloaded) {
            Icon(
                Icons.Default.DownloadForOffline, stringResource(R.string.downloaded), Modifier.size(16.dp),
                tint = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            Spacer(Modifier.width(6.dp))
        }
        track.durationSecs?.let {
            Text(formatTime(it.toDouble()), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
        Box {
            IconButton(onClick = { menu = true }) { Icon(Icons.Default.MoreVert, stringResource(R.string.more)) }
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
    Row(
        Modifier.fillMaxWidth().clickable { Router.open(Route.Page(item.browseId)) }.padding(horizontal = 16.dp, vertical = 6.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Artwork(item.art, 56.dp, round = item.kind == ItemKind.Artist)
        Spacer(Modifier.width(12.dp))
        Column(Modifier.weight(1f)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(item.title, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f, fill = false))
                if (item.explicit) {
                    Spacer(Modifier.width(6.dp))
                    ExplicitBadge()
                }
            }
            Text(
                item.subtitle, maxLines = 1, overflow = TextOverflow.Ellipsis,
                style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}

/** An Explore chip ("Chill", "Charts"): a coloured tile, no picture. */
@Composable
private fun Chip(item: Item) {
    val hue = (item.title.hashCode() and 0xFFFF) % 360
    Box(
        Modifier
            .width(150.dp)
            .height(64.dp)
            .clip(RoundedCornerShape(10.dp))
            .background(Color.hsv(hue.toFloat(), 0.55f, 0.45f))
            .clickable { Router.open(Route.Page(item.browseId)) }
            .padding(12.dp),
        contentAlignment = Alignment.BottomStart,
    ) {
        Text(item.title, fontWeight = FontWeight.SemiBold, maxLines = 2, overflow = TextOverflow.Ellipsis, color = Color.White)
    }
}

@OptIn(ExperimentalFoundationApi::class)
@Composable
fun Card(item: Item, songs: List<Track>) {
    if (item.track == null && item.thumbnail == null && item.subtitle.isEmpty()) return Chip(item)
    var menu by remember { mutableStateOf(false) }
    val track = item.track
    Column(
        Modifier
            .width(150.dp)
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
    ) {
        Box {
            Artwork(item.art, 150.dp, round = item.kind == ItemKind.Artist)
            if (item.explicit) Box(Modifier.align(Alignment.BottomStart).padding(6.dp)) { ExplicitBadge() }
            if (track != null) TrackMenu(track, menu, { menu = false })
        }
        Spacer(Modifier.height(6.dp))
        Text(
            item.title, style = MaterialTheme.typography.bodyMedium, fontWeight = FontWeight.Medium,
            maxLines = 2, overflow = TextOverflow.Ellipsis,
        )
        Text(
            item.subtitle, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant,
            maxLines = 2, overflow = TextOverflow.Ellipsis,
        )
    }
}

@Composable
fun ShelfView(shelf: Shelf, showTitle: Boolean = true) {
    Column {
        if (showTitle && shelf.title.isNotEmpty()) {
            Text(
                shelf.title, style = MaterialTheme.typography.titleLarge, fontWeight = FontWeight.Bold,
                modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
            )
        }
        val songs = shelf.items.mapNotNull { it.track }
        LazyRow(contentPadding = PaddingValues(horizontal = 16.dp), horizontalArrangement = Arrangement.spacedBy(14.dp)) {
            items(shelf.items, key = { it.id }) { Card(it, songs) }
        }
    }
}

fun moveActions(up: (() -> Unit)?, down: (() -> Unit)?, upLabel: String, downLabel: String): List<MenuAction> =
    listOfNotNull(
        up?.let { MenuAction(upLabel, Icons.Default.ArrowUpward, it) },
        down?.let { MenuAction(downLabel, Icons.Default.ArrowDownward, it) },
    )

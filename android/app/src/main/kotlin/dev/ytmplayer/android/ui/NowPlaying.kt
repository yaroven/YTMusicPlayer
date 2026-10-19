package dev.ytmplayer.android.ui

import android.os.Build
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.spring
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.rememberScrollState
import androidx.compose.material.icons.automirrored.filled.PlaylistAdd
import androidx.compose.material.icons.filled.Download
import androidx.compose.material.icons.filled.DownloadDone
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material.icons.filled.Share
import androidx.compose.material.icons.filled.ThumbUp
import androidx.compose.material.icons.outlined.ThumbDown
import androidx.compose.material.icons.outlined.ThumbUp
import androidx.compose.ui.platform.LocalContext
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.QueueMusic
import androidx.compose.material.icons.filled.Bedtime
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.KeyboardArrowUp
import androidx.compose.material.icons.filled.ExpandMore
import androidx.compose.material.icons.filled.Favorite
import androidx.compose.material.icons.filled.FavoriteBorder
import androidx.compose.material.icons.filled.Lyrics
import androidx.compose.material.icons.filled.MusicNote
import androidx.compose.material.icons.filled.Pause
import androidx.compose.material.icons.filled.PlayArrow
import androidx.compose.material.icons.filled.Repeat
import androidx.compose.material.icons.filled.RepeatOne
import androidx.compose.material.icons.filled.Shuffle
import androidx.compose.material.icons.filled.SkipNext
import androidx.compose.material.icons.filled.SkipPrevious
import androidx.compose.material.icons.filled.SmartDisplay
import androidx.compose.material.icons.filled.ThumbDownOffAlt
import androidx.compose.material.icons.filled.ViewCarousel
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Slider
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableDoubleStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.blur
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.scale
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.ytmplayer.android.App
import dev.ytmplayer.android.R
import dev.ytmplayer.android.RepeatMode
import dev.ytmplayer.android.formatTime
import dev.ytmplayer.core.Lyrics
import dev.ytmplayer.core.LyricsProgress
import dev.ytmplayer.core.Track
import kotlinx.coroutines.delay

private enum class Pane { Lyrics, Queue, Related }

@Composable
fun NowPlaying() {
    val player = App.instance.player
    var pane by remember { mutableStateOf<Pane?>(null) }
    // The clip instead of the cover; where it started and where it got.
    var video by remember { mutableStateOf(false) }
    var videoStart by remember { mutableDoubleStateOf(0.0) }
    var videoAt by remember { mutableStateOf<Double?>(null) }
    val track = player.current

    fun setVideo(on: Boolean) {
        if (on == video) return
        video = on
        if (on) {
            videoAt = null
            videoStart = player.pauseForVideo()
        } else {
            player.resumeAfterVideo(videoAt)
        }
    }
    // Another song: back to its audio, from its start.
    LaunchedEffect(track?.videoId) { if (video) { video = false; videoAt = null } }

    Surface(Modifier.fillMaxSize(), color = Color.Black, contentColor = Color.White) {
    Box(Modifier.fillMaxSize()) {
        if (track != null) Backdrop(track)
        Column(
            Modifier.fillMaxSize().statusBarsPadding().navigationBarsPadding().padding(horizontal = 24.dp),
            verticalArrangement = Arrangement.spacedBy(14.dp),
        ) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                IconButton(onClick = {
                    setVideo(false)
                    player.showNowPlaying = false
                }) { Icon(Icons.Default.ExpandMore, stringResource(R.string.close), Modifier.size(30.dp)) }
                Spacer(Modifier.weight(1f))
                if (track != null && pane == null) MediaSwitch(video, ::setVideo)
                Spacer(Modifier.weight(1f))
                if (track != null) {
                    Box {
                        var menu by remember { mutableStateOf(false) }
                        IconButton(onClick = { menu = true }) { Icon(Icons.Default.MoreVert, stringResource(R.string.more)) }
                        TrackMenu(track, menu, { menu = false })
                    }
                } else {
                    Spacer(Modifier.width(48.dp))
                }
            }
            if (track == null) {
                Message(Icons.Default.MusicNote, stringResource(R.string.nothing_playing))
                return@Column
            }
            Box(Modifier.weight(1f).fillMaxWidth()) {
                when (pane) {
                    null -> if (video) {
                        VideoPane(track.videoId, videoStart) { videoAt = it }
                    } else {
                        Cover(track)
                    }
                    Pane.Lyrics -> LyricsPane(track)
                    Pane.Queue -> QueuePane()
                    Pane.Related -> RelatedPane(track)
                }
            }
            Header(track)
            Actions(track)
            Scrubber()
            Controls()
            // YouTube Music's UP NEXT / LYRICS / RELATED, plus the sleep timer.
            Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                listOf(Pane.Queue to R.string.up_next, Pane.Lyrics to R.string.lyrics, Pane.Related to R.string.related).forEach { (target, label) ->
                    Box(
                        Modifier.weight(1f).height(40.dp).clip(RoundedCornerShape(8.dp))
                            .clickable { pane = if (pane == target) null else target },
                        contentAlignment = Alignment.Center,
                    ) {
                        Text(
                            stringResource(label).uppercase(), fontSize = 13.sp, fontWeight = FontWeight.Bold, letterSpacing = 0.6.sp,
                            color = if (pane == target) Color.White else YT.secondary,
                        )
                    }
                }
                SleepMenu()
            }
            Spacer(Modifier.height(4.dp))
        }
    }
    }
}

/** The cover, blurred, behind everything (Android 12+ can blur). */
@Composable
private fun Backdrop(track: Track) {
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
        Box(Modifier.fillMaxSize().graphicsLayer { alpha = 0.55f }) {
            coil.compose.AsyncImage(
                model = "https://i.ytimg.com/vi/${track.videoId}/mqdefault.jpg", contentDescription = null,
                contentScale = androidx.compose.ui.layout.ContentScale.Crop,
                modifier = Modifier.fillMaxSize().scale(1.4f).blur(60.dp),
            )
        }
    }
    Box(Modifier.fillMaxSize().background(Brush.verticalGradient(listOf(Color.Black.copy(alpha = 0.2f), Color.Black.copy(alpha = 0.75f)))))
}

/** YouTube Music's ♪ / ▶ switch over the cover. */
@Composable
private fun MediaSwitch(video: Boolean, onChange: (Boolean) -> Unit) {
    Row(
        Modifier.background(Color.White.copy(alpha = 0.1f), CircleShape).padding(4.dp),
        horizontalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        listOf(false to Icons.Default.MusicNote, true to Icons.Default.SmartDisplay).forEach { (value, icon) ->
            Box(
                Modifier
                    .clip(CircleShape)
                    .background(if (video == value) Color.White.copy(alpha = 0.15f) else Color.Transparent)
                    .clickable { onChange(value) }
                    .padding(horizontal = 14.dp, vertical = 6.dp),
            ) {
                Icon(
                    icon, stringResource(if (value) R.string.video else R.string.song),
                    tint = if (video == value) Color.White else Color.White.copy(alpha = 0.5f),
                )
            }
        }
    }
}

@Composable
private fun Cover(track: Track) {
    val player = App.instance.player
    val scale by animateFloatAsState(if (player.isPlaying) 1f else 0.86f, spring(dampingRatio = 0.7f), label = "cover")
    BoxWithConstraints(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
        val side = minOf(maxWidth, maxHeight)
        // The cover itself plays and pauses.
        Box(Modifier.scale(scale).clickable { player.togglePlay() }) {
            Artwork(Art.Video(track.videoId), side)
        }
    }
}

@Composable
private fun Header(track: Track) {
    Column(Modifier.fillMaxWidth()) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(
                track.title, fontSize = 22.sp, fontWeight = FontWeight.Bold,
                maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f, fill = false),
            )
            if (track.explicit) {
                Spacer(Modifier.width(8.dp))
                ExplicitBadge()
            }
        }
        Text(
            track.artist, fontSize = 16.sp, maxLines = 1, overflow = TextOverflow.Ellipsis, color = YT.secondary,
            modifier = Modifier.clickable { Router.openArtist(track) },
        )
        Status()
    }
}

/** 👍 | 👎, Save, Download, Share — YouTube Music's pill row. */
@Composable
private fun Actions(track: Track) {
    val app = App.instance
    val library = app.library
    val context = LocalContext.current
    var saving by remember { mutableStateOf(false) }
    Row(
        Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()),
        horizontalArrangement = Arrangement.spacedBy(8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        val liked = library.isLiked(track)
        Row(Modifier.height(36.dp).clip(CircleShape).background(YT.raised), verticalAlignment = Alignment.CenterVertically) {
            Box(Modifier.fillMaxHeight().clickable { library.toggleLike(track) }.padding(horizontal = 14.dp), contentAlignment = Alignment.Center) {
                Icon(if (liked) Icons.Filled.ThumbUp else Icons.Outlined.ThumbUp, stringResource(R.string.like), Modifier.size(20.dp))
            }
            Box(Modifier.width(1.dp).height(18.dp).background(Color.White.copy(alpha = 0.25f)))
            Box(Modifier.fillMaxHeight().clickable { app.player.dislike(track) }.padding(horizontal = 14.dp), contentAlignment = Alignment.Center) {
                Icon(Icons.Outlined.ThumbDown, stringResource(R.string.dislike), Modifier.size(20.dp))
            }
        }
        Pill(Icons.AutoMirrored.Filled.PlaylistAdd, stringResource(R.string.save)) { saving = true }
        when {
            library.isDownloaded(track.videoId) ->
                Pill(Icons.Default.DownloadDone, stringResource(R.string.downloaded)) { library.removeDownload(track) }
            track.videoId in library.downloading -> Pill(Icons.Default.Download, stringResource(R.string.downloading)) {}
            else -> Pill(Icons.Default.Download, stringResource(R.string.download)) { library.download(track) }
        }
        Pill(Icons.Default.Share, stringResource(R.string.share)) {
            val send = android.content.Intent(android.content.Intent.ACTION_SEND)
                .setType("text/plain")
                .putExtra(android.content.Intent.EXTRA_TEXT, "https://music.youtube.com/watch?v=${track.videoId}")
            context.startActivity(android.content.Intent.createChooser(send, null))
        }
    }
    if (saving) PlaylistChooser(track) { saving = false }
}

@Composable
private fun Pill(icon: androidx.compose.ui.graphics.vector.ImageVector, label: String, onClick: () -> Unit) {
    Row(
        Modifier.height(36.dp).clip(CircleShape).background(YT.raised).clickable(onClick = onClick).padding(horizontal = 14.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(icon, null, Modifier.size(18.dp))
        Spacer(Modifier.width(6.dp))
        Text(label, fontSize = 14.sp, fontWeight = FontWeight.Medium)
    }
}

/** What the player is waiting for, or what went wrong. */
@Composable
fun Status(compact: Boolean = false) {
    val player = App.instance.player
    val online by App.instance.network.online.collectAsState()
    val text = when {
        player.error != null -> player.error
        player.waitingForNetwork -> stringResource(R.string.player_waiting_offline)
        player.slow && player.isPlaying -> stringResource(if (online) R.string.player_slow else R.string.player_waiting_offline)
        else -> null
    } ?: return
    Text(
        text,
        style = MaterialTheme.typography.bodySmall,
        color = if (player.error != null) Color(0xFFFF6B6B) else MaterialTheme.colorScheme.onSurfaceVariant,
        maxLines = if (compact) 1 else 2,
        overflow = TextOverflow.Ellipsis,
    )
}

/** Position slider; its own composable so ticks don't redraw the whole screen. */
@Composable
private fun Scrubber() {
    val player = App.instance.player
    var dragging by remember { mutableStateOf<Float?>(null) }
    val shown = dragging?.toDouble() ?: player.position
    Column {
        Slider(
            value = shown.toFloat().coerceIn(0f, maxOf(player.duration.toFloat(), 1f)),
            onValueChange = { dragging = it },
            onValueChangeFinished = {
                dragging?.let { player.seek(it.toDouble()) }
                dragging = null
            },
            valueRange = 0f..maxOf(player.duration.toFloat(), 1f),
            colors = ytSlider(),
        )
        Row {
            Text(formatTime(shown), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            Spacer(Modifier.weight(1f))
            Text(
                "-" + formatTime(maxOf(0.0, player.duration - shown)), style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}

@Composable
private fun Controls() {
    val player = App.instance.player
    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween, verticalAlignment = Alignment.CenterVertically) {
        IconButton(onClick = { player.shuffleUpcoming() }) { Icon(Icons.Default.Shuffle, stringResource(R.string.shuffle)) }
        IconButton(onClick = { player.previous() }, Modifier.size(56.dp)) {
            Icon(Icons.Default.SkipPrevious, stringResource(R.string.previous), Modifier.size(36.dp))
        }
        Box(
            Modifier.size(72.dp).clip(CircleShape).background(Color.White).clickable { player.togglePlay() },
            contentAlignment = Alignment.Center,
        ) {
            if (player.isBuffering && player.isPlaying) {
                CircularProgressIndicator(Modifier.size(32.dp), color = Color.Black, strokeWidth = 3.dp)
            } else {
                Icon(
                    if (player.isPlaying) Icons.Default.Pause else Icons.Default.PlayArrow,
                    stringResource(if (player.isPlaying) R.string.pause else R.string.play),
                    Modifier.size(40.dp), tint = Color.Black,
                )
            }
        }
        IconButton(onClick = { player.next() }, Modifier.size(56.dp)) {
            Icon(Icons.Default.SkipNext, stringResource(R.string.next), Modifier.size(36.dp))
        }
        IconButton(onClick = { player.cycleRepeat() }) {
            Icon(
                if (player.repeatMode == RepeatMode.One) Icons.Default.RepeatOne else Icons.Default.Repeat,
                stringResource(R.string.repeat),
                tint = if (player.repeatMode == RepeatMode.Off) YT.secondary else Color.White,
            )
        }
    }
}

@Composable
private fun PaneToggle(icon: androidx.compose.ui.graphics.vector.ImageVector, label: String, on: Boolean, onClick: () -> Unit) {
    IconButton(onClick = onClick) {
        Icon(icon, label, tint = if (on) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurfaceVariant)
    }
}

/** Sleep timer: pause after a while, or when the song ends. */
@Composable
private fun SleepMenu() {
    val player = App.instance.player
    var open by remember { mutableStateOf(false) }
    val on = player.sleepUntil != null || player.sleepAtEndOfSong
    Box {
        PaneToggle(Icons.Default.Bedtime, stringResource(R.string.sleep_timer), on) { open = true }
        DropdownMenu(open, { open = false }) {
            listOf(15, 30, 60).forEach { minutes ->
                DropdownMenuItem(text = { Text(stringResource(R.string.minutes, minutes)) }, onClick = {
                    open = false
                    player.setSleep(minutes)
                })
            }
            DropdownMenuItem(text = { Text(stringResource(R.string.end_of_song)) }, onClick = {
                open = false
                player.setSleep(null)
            })
            if (on) {
                DropdownMenuItem(text = { Text(stringResource(R.string.turn_off)) }, onClick = {
                    open = false
                    player.cancelSleep()
                })
            }
        }
    }
}

/** Karaoke lyrics: big bold lines, the sung one lighting up word by word; a tap seeks to a line. */
@Composable
private fun LyricsPane(track: Track) {
    val app = App.instance
    val player = app.player
    val lyrics = rememberLoad(track.videoId) {
        val duration = track.durationSecs?.toDouble() ?: player.duration.takeIf { it > 0 }
        app.lyrics.find(track, duration) ?: NoLyrics
    }
    Loaded(lyrics) { found ->
        if (found === NoLyrics) {
            Message(Icons.Default.Lyrics, stringResource(R.string.no_lyrics))
        } else if (found.synced) {
            SyncedLyrics(found)
        } else {
            LazyColumn(contentPadding = PaddingValues(vertical = 24.dp)) {
                items(found.lines) { Text(it.text, style = MaterialTheme.typography.titleMedium) }
                item { Text(found.source, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant) }
            }
        }
    }
}

private val NoLyrics = Lyrics(emptyList(), "")

@Composable
private fun SyncedLyrics(lyrics: Lyrics) {
    val player = App.instance.player
    var progress by remember { mutableStateOf<LyricsProgress?>(null) }
    // Ten times a second while on screen: enough for words to light up on time.
    LaunchedEffect(lyrics) {
        while (true) {
            progress = lyrics.progress(player.exactPosition + 0.15)
            delay(100)
        }
    }
    val list = rememberLazyListState()
    val offset = if (lyrics.hasIntro) 1 else 0
    LaunchedEffect(progress?.line) {
        val line = progress?.line ?: return@LaunchedEffect
        list.animateScrollToItem(maxOf(0, line + offset - 1))
    }
    LazyColumn(state = list, contentPadding = PaddingValues(top = 24.dp, bottom = 280.dp), verticalArrangement = Arrangement.spacedBy(20.dp)) {
        if (lyrics.hasIntro) item { Dots(if (progress?.line == -1) progress?.words ?: 0 else 0, progress?.line == -1) }
        itemsIndexed(lyrics.lines) { i, line ->
            val p = progress
            val current = p?.line == i
            val sung = p != null && i < p.line
            Box(Modifier.fillMaxWidth().clickable { line.at?.let(player::seek) }) {
                when {
                    line.instrumental -> Dots(if (current) p?.words ?: 0 else 0, current)
                    current -> {
                        val lit = minOf(p?.chars ?: 0, line.text.length)
                        Text(
                            buildAnnotatedString {
                                withStyle(SpanStyle(color = Color.White)) { append(line.text.take(lit)) }
                                withStyle(SpanStyle(color = Color.White.copy(alpha = 0.38f))) { append(line.text.drop(lit)) }
                            },
                            fontSize = 26.sp, fontWeight = FontWeight.Bold, lineHeight = 32.sp,
                        )
                    }
                    else -> Text(
                        line.text, fontSize = 26.sp, fontWeight = FontWeight.Bold, lineHeight = 32.sp,
                        color = Color.White.copy(alpha = if (sung) 0.22f else 0.38f),
                    )
                }
            }
        }
        item { Text(lyrics.source, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant) }
    }
}

/** An instrumental break: three dots lighting up until the next line. */
@Composable
private fun Dots(lit: Int, active: Boolean) {
    Row(Modifier.height(34.dp), horizontalArrangement = Arrangement.spacedBy(10.dp), verticalAlignment = Alignment.CenterVertically) {
        repeat(3) { d ->
            Box(
                Modifier.size(if (active) 13.dp else 9.dp).clip(CircleShape)
                    .background(Color.White.copy(alpha = if (active && d < lit) 1f else 0.25f)),
            )
        }
    }
}

@Composable
private fun QueuePane() {
    val player = App.instance.player
    val upNext = player.upNext
    if (upNext.isEmpty()) {
        Message(
            Icons.AutoMirrored.Filled.QueueMusic, stringResource(R.string.queue_empty),
            if (player.autoplay) stringResource(R.string.similar_will_follow) else null,
        )
        return
    }
    var naming by remember { mutableStateOf(false) }
    LazyColumn {
        item {
            Row(Modifier.fillMaxWidth().padding(vertical = 8.dp), verticalAlignment = Alignment.CenterVertically) {
                Text(stringResource(R.string.up_next), Modifier.weight(1f), fontSize = 18.sp, fontWeight = FontWeight.Bold)
                // The whole queue (what played, the song, what's next) as a playlist.
                OutlinePill(stringResource(R.string.save_queue)) { naming = true }
            }
        }
        itemsIndexed(upNext) { i, track ->
            Row(
                Modifier.fillMaxWidth().clickable { player.jump(player.index + 1 + i) }.padding(vertical = 4.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Artwork(Art.Video(track.videoId), 40.dp)
                Spacer(Modifier.width(12.dp))
                Column(Modifier.weight(1f)) {
                    Text(track.title, maxLines = 1, overflow = TextOverflow.Ellipsis)
                    Text(
                        track.artist, maxLines = 1, overflow = TextOverflow.Ellipsis,
                        style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                if (i > 0) {
                    IconButton(onClick = { player.moveUpcoming(i, i - 1) }) {
                        Icon(Icons.Default.KeyboardArrowUp, stringResource(R.string.move_up))
                    }
                }
                IconButton(onClick = { player.removeUpcoming(i) }) { Icon(Icons.Default.Close, stringResource(R.string.remove)) }
            }
        }
    }
    if (naming) SaveQueueDialog { naming = false }
}

/** Names the queue and saves it as a playlist (on YouTube too, signed in). */
@Composable
private fun SaveQueueDialog(onDismiss: () -> Unit) {
    val app = App.instance
    val date = java.text.DateFormat.getDateInstance(java.text.DateFormat.MEDIUM).format(java.util.Date())
    NameDialog(stringResource(R.string.save_queue), stringResource(R.string.queue_playlist_name, date), onDismiss) {
        app.library.createPlaylist(it, app.player.queue)
        onDismiss()
    }
}

/** Songs, albums and artists YouTube Music relates to the playing song. */
@Composable
private fun RelatedPane(track: Track) {
    val related = rememberLoad(track.videoId) { App.instance.catalog.related(track.videoId) }
    Loaded(related) { shelves ->
        if (shelves.isEmpty()) {
            Message(Icons.Default.ViewCarousel, stringResource(R.string.nothing_related))
        } else {
            LazyColumn(verticalArrangement = Arrangement.spacedBy(18.dp)) {
                items(shelves, key = { it.id }) { ShelfView(it) }
            }
        }
    }
}

/** Above the tab bar: the song, play / pause, next; a tap opens the player. */
@Composable
fun MiniPlayer() {
    val player = App.instance.player
    val track = player.current ?: return
    Column(Modifier.fillMaxWidth().background(YT.surface).clickable { player.showNowPlaying = true }) {
        // Progress line along the top.
        val fraction = if (player.duration > 0) (player.position / player.duration).toFloat().coerceIn(0f, 1f) else 0f
        Box(Modifier.fillMaxWidth().height(2.dp).background(Color.White.copy(alpha = 0.15f))) {
            Box(Modifier.fillMaxWidth(fraction).height(2.dp).background(Color.White))
        }
        Row(Modifier.padding(start = 10.dp, top = 6.dp, bottom = 6.dp), verticalAlignment = Alignment.CenterVertically) {
            Artwork(Art.Video(track.videoId), 44.dp)
            Spacer(Modifier.width(12.dp))
            Column(Modifier.weight(1f)) {
                Text(track.title, fontWeight = FontWeight.SemiBold, fontSize = 15.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
                if (player.error != null || player.waitingForNetwork || (player.slow && player.isPlaying)) {
                    Status(compact = true)
                } else {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        if (track.explicit) {
                            ExplicitBadge()
                            Spacer(Modifier.width(5.dp))
                        }
                        Text(track.artist, fontSize = 13.sp, color = YT.secondary, maxLines = 1, overflow = TextOverflow.Ellipsis)
                    }
                }
            }
            IconButton(onClick = { player.togglePlay() }) {
                if (player.isBuffering && player.isPlaying) {
                    CircularProgressIndicator(Modifier.size(22.dp), strokeWidth = 2.dp, color = Color.White)
                } else {
                    Icon(
                        if (player.isPlaying) Icons.Default.Pause else Icons.Default.PlayArrow,
                        stringResource(if (player.isPlaying) R.string.pause else R.string.play),
                        Modifier.size(30.dp),
                    )
                }
            }
            IconButton(onClick = { player.next() }) { Icon(Icons.Default.SkipNext, stringResource(R.string.next), Modifier.size(28.dp)) }
        }
    }
}

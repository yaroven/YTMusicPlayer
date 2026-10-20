package dev.yarek.music.ui

import androidx.compose.runtime.rememberCoroutineScope
import kotlinx.coroutines.launch
import androidx.compose.animation.core.tween
import androidx.compose.animation.core.Animatable
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.material3.rememberSwipeToDismissBoxState
import androidx.compose.material3.TextButton
import androidx.compose.material3.SwipeToDismissBoxValue
import androidx.compose.material3.SwipeToDismissBox
import androidx.compose.material3.AlertDialog
import androidx.compose.material.icons.outlined.Bedtime
import androidx.compose.material.icons.filled.DragHandle
import androidx.compose.foundation.gestures.detectVerticalDragGestures
import androidx.compose.foundation.gestures.detectDragGestures
import androidx.compose.foundation.basicMarquee
import android.os.Build
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.spring
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectHorizontalDragGestures
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.offset
import androidx.compose.material.icons.filled.ChevronRight
import androidx.compose.material.icons.filled.FormatQuote
import androidx.compose.material.icons.filled.Headphones
import androidx.compose.material.icons.filled.Sensors
import androidx.compose.material.icons.outlined.SmartDisplay
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.foundation.horizontalScroll
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
import dev.yarek.music.App
import dev.yarek.music.R
import dev.yarek.music.RepeatMode
import dev.yarek.music.formatTime
import dev.yarek.core.Lyrics
import dev.yarek.core.LyricsProgress
import dev.yarek.core.Track
import kotlinx.coroutines.delay

private enum class Pane(val label: Int) { Queue(R.string.your_queue), Related(R.string.related) }

/**
 * YouTube Music's player: ⌄, the song / video switch, ⋮; the big cover (or
 * the clip); the title with its ›, the artist; a row of pills (👍|👎,
 * Lyrics, Save, Share, Download, Mix); a thin scrubber; the controls; and
 * "Up next" at the bottom, a sheet with the queue, the lyrics and related
 * music.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun NowPlaying() {
    val player = App.instance.player
    var sheet by remember { mutableStateOf<Pane?>(null) }
    var lyricsOpen by remember { mutableStateOf(false) }
    // The clip instead of the cover; where it started and where it got.
    var video by remember { mutableStateOf(false) }
    var videoStart by remember { mutableDoubleStateOf(0.0) }
    var videoAt by remember { mutableStateOf<Double?>(null) }
    // The song and its clip (Pairs): "Video" only when there is a clip,
    // started where the song is (its intro added).
    var pair by remember { mutableStateOf<dev.yarek.core.MediaPair?>(null) }
    LaunchedEffect(player.current?.videoId) {
        pair = null
        val now = player.current ?: return@LaunchedEffect
        pair = dev.yarek.music.Pairs.pair(now, findClip = true)
    }
    val offset = pair?.offset ?: 0.0
    val track = player.current

    fun setVideo(on: Boolean) {
        if (on == video) return
        video = on
        if (on) {
            videoAt = null
            videoStart = player.pauseForVideo()
        } else {
            // Clip time → song time (the clip's intro isn't in the song).
            player.resumeAfterVideo(videoAt?.let { maxOf(0.0, it - offset) })
        }
    }
    // Another song: back to its audio, from its start.
    LaunchedEffect(track?.videoId) { if (video) { video = false; videoAt = null } }

    // Swipe down anywhere (but the scrubber, pills and cover, which move
    // sideways) folds the player away, as in YouTube Music.
    val scope = rememberCoroutineScope()
    val pull = remember { Animatable(0f) }
    Surface(
        Modifier
            .fillMaxSize()
            .graphicsLayer { translationY = pull.value }
            .pointerInput(Unit) {
                detectVerticalDragGestures(
                    onDragEnd = {
                        scope.launch {
                            if (pull.value > size.height * 0.18f) {
                                setVideo(false)
                                player.showNowPlaying = false
                                pull.snapTo(0f)
                            } else {
                                pull.animateTo(0f, spring(dampingRatio = 0.8f))
                            }
                        }
                    },
                    onDragCancel = { scope.launch { pull.animateTo(0f) } },
                ) { change, amount ->
                    change.consume()
                    scope.launch { pull.snapTo((pull.value + amount).coerceAtLeast(0f)) }
                }
            },
        color = Color.Black, contentColor = Color.White,
    ) {
    Box(Modifier.fillMaxSize()) {
        if (track != null) Backdrop(track)
        Column(
            Modifier.fillMaxSize().statusBarsPadding().navigationBarsPadding(),
            verticalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            Row(Modifier.padding(horizontal = 8.dp), verticalAlignment = Alignment.CenterVertically) {
                IconButton(onClick = {
                    setVideo(false)
                    player.showNowPlaying = false
                }) { Icon(Icons.Default.ExpandMore, stringResource(R.string.close), Modifier.size(32.dp)) }
                Spacer(Modifier.weight(1f))
                // Only songs with a clip offer "Video" (YouTube Music's switch).
                if (track != null && (pair?.video != null || video)) MediaSwitch(video, ::setVideo)
                Spacer(Modifier.weight(1f))
                if (track != null) {
                    Box {
                        var menu by remember { mutableStateOf(false) }
                        var sleep by remember { mutableStateOf(false) }
                        val sleepOn = player.sleepUntil != null || player.sleepAtEndOfSong
                        IconButton(onClick = { menu = true }) { Icon(Icons.Default.MoreVert, stringResource(R.string.more)) }
                        TrackMenu(
                            track, menu, { menu = false },
                            listOf(MenuAction(stringResource(R.string.sleep_timer), if (sleepOn) Icons.Default.Bedtime else Icons.Outlined.Bedtime) { sleep = true }),
                        )
                        if (sleep) SleepDialog { sleep = false }
                    }
                } else {
                    Spacer(Modifier.width(48.dp))
                }
            }
            if (track == null) {
                Message(Icons.Default.MusicNote, stringResource(R.string.nothing_playing))
                return@Column
            }
            Box(Modifier.weight(1f).fillMaxWidth().padding(horizontal = 32.dp)) {
                val clip = pair?.video
                if (video && clip != null) VideoPane(clip, videoStart + offset) { videoAt = it } else Cover(track)
            }
            Header(track)
            Actions(track) { lyricsOpen = true }
            Scrubber()
            Controls()
            // "Your queue": the sheet with the queue and related music.
            Box(
                Modifier.fillMaxWidth().height(48.dp)
                    .clickable { sheet = Pane.Queue }
                    .pointerInput(Unit) { detectVerticalDragGestures { _, drag -> if (drag < -8) sheet = Pane.Queue } },
                contentAlignment = Alignment.Center,
            ) {
                Text(stringResource(R.string.your_queue).uppercase(), fontSize = 14.sp, fontWeight = FontWeight.Medium, letterSpacing = 0.5.sp)
            }
        }
    }
    }

    if (lyricsOpen && track != null) LyricsSheet(track) { lyricsOpen = false }
    sheet?.let { start ->
        val state = rememberModalBottomSheetState(skipPartiallyExpanded = true)
        ModalBottomSheet(
            onDismissRequest = { sheet = null }, sheetState = state,
            containerColor = SHEET, contentColor = Color.White,
            dragHandle = null,
        ) {
            var tab by remember { mutableStateOf(start) }
            Column(Modifier.fillMaxWidth().fillMaxHeight(0.86f)) {
                Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                    Pane.entries.forEach { p ->
                        Column(
                            Modifier.weight(1f).clickable { tab = p }.padding(top = 16.dp),
                            horizontalAlignment = Alignment.CenterHorizontally,
                        ) {
                            Text(
                                stringResource(p.label).uppercase(), fontSize = 14.sp, fontWeight = FontWeight.Medium,
                                letterSpacing = 0.5.sp, color = if (tab == p) Color.White else YT.secondary,
                            )
                            Spacer(Modifier.height(12.dp))
                            Box(Modifier.fillMaxWidth(0.6f).height(2.dp).background(if (tab == p) Color.White else Color.Transparent))
                        }
                    }
                }
                Box(Modifier.padding(horizontal = 16.dp).fillMaxWidth().height(1.dp).background(Color.White.copy(alpha = 0.1f)))
                Box(Modifier.weight(1f).fillMaxWidth()) {
                    when (tab) {
                        Pane.Queue -> QueuePane()
                        Pane.Related -> Box(Modifier.padding(top = 12.dp)) { RelatedPane(track ?: return@Box) }
                    }
                }
            }
        }
    }
}

/** "Lyrics": YouTube Music's own sheet — a title, ✕, the lines over the blurred cover, Share. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun LyricsSheet(track: Track, onDismiss: () -> Unit) {
    val context = LocalContext.current
    ModalBottomSheet(
        onDismissRequest = onDismiss, sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
        containerColor = SHEET, contentColor = Color.White,
    ) {
        Column(Modifier.fillMaxWidth().fillMaxHeight(0.86f)) {
            Row(Modifier.fillMaxWidth().padding(start = 16.dp), verticalAlignment = Alignment.CenterVertically) {
                Text(stringResource(R.string.lyrics), Modifier.weight(1f), fontSize = 18.sp, fontWeight = FontWeight.Bold)
                IconButton(onClick = onDismiss) { Icon(Icons.Default.Close, stringResource(R.string.close)) }
            }
            Box(Modifier.weight(1f).fillMaxWidth().clipToBounds()) {
                Backdrop(track)
                Box(Modifier.padding(horizontal = 20.dp)) { LyricsPane(track) }
                Row(
                    Modifier.align(Alignment.BottomCenter).padding(bottom = 16.dp)
                        .clip(CircleShape).background(Color(0xFFF1F1F1))
                        .clickable {
                            val send = android.content.Intent(android.content.Intent.ACTION_SEND)
                                .setType("text/plain")
                                .putExtra(android.content.Intent.EXTRA_TEXT, "https://music.youtube.com/watch?v=${track.videoId}")
                            context.startActivity(android.content.Intent.createChooser(send, null))
                        }
                        .padding(horizontal = 20.dp, vertical = 12.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Icon(Icons.Default.Share, null, Modifier.size(20.dp), tint = Color.Black)
                    Spacer(Modifier.width(8.dp))
                    Text(stringResource(R.string.share), color = Color.Black, fontWeight = FontWeight.Medium)
                }
            }
        }
    }
}

/** The sheets' grey. */
private val SHEET = Color(0xFF212121)

/** The pills' grey (YouTube Music's #272727). */
private val PILL = Color(0xFF272727)

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

/** YouTube Music's "Song | Video" switch over the cover. */
@Composable
private fun MediaSwitch(video: Boolean, onChange: (Boolean) -> Unit) {
    Row(Modifier.clip(CircleShape).background(Color.White.copy(alpha = 0.1f)).padding(2.dp)) {
        listOf(false to R.string.song, true to R.string.video).forEach { (value, label) ->
            Box(
                Modifier
                    .width(70.dp)
                    .height(32.dp)
                    .clip(CircleShape)
                    .background(if (video == value) Color.White.copy(alpha = 0.2f) else Color.Transparent)
                    .clickable { onChange(value) },
                contentAlignment = Alignment.Center,
            ) {
                Text(
                    stringResource(label), fontSize = 14.sp, fontWeight = FontWeight.Medium,
                    color = if (video == value) Color.White else Color.White.copy(alpha = 0.7f),
                )
            }
        }
    }
}

@Composable
private fun Cover(track: Track) {
    val player = App.instance.player
    val scale by animateFloatAsState(if (player.isPlaying) 1f else 0.86f, spring(dampingRatio = 0.7f), label = "cover")
    val scope = rememberCoroutineScope()
    // YouTube Music's swipe on the cover: it follows the finger; far enough
    // and it slides out, the next (left) or previous (right) song slides in.
    val offset = remember { Animatable(0f) }
    BoxWithConstraints(
        Modifier.fillMaxSize().pointerInput(Unit) {
            val width = size.width.toFloat()
            detectHorizontalDragGestures(
                onDragEnd = {
                    scope.launch {
                        val x = offset.value
                        if (kotlin.math.abs(x) < width * 0.25f) {
                            offset.animateTo(0f, spring(dampingRatio = 0.8f))
                            return@launch
                        }
                        val forward = x < 0
                        offset.animateTo(if (forward) -width else width, tween(140))
                        if (forward) player.next() else player.previous(always = true)
                        offset.snapTo(if (forward) width else -width)
                        offset.animateTo(0f, tween(220))
                    }
                },
                onDragCancel = { scope.launch { offset.animateTo(0f) } },
            ) { change, amount ->
                change.consume()
                scope.launch { offset.snapTo(offset.value + amount) }
            }
        },
        contentAlignment = Alignment.Center,
    ) {
        val side = minOf(maxWidth, maxHeight)
        val width = constraints.maxWidth.toFloat().coerceAtLeast(1f)
        // The cover itself plays and pauses.
        Box(
            Modifier
                .graphicsLayer {
                    translationX = offset.value
                    alpha = 1f - (kotlin.math.abs(offset.value) / width * 0.6f).coerceIn(0f, 0.6f)
                }
                .scale(scale)
                .clickable { player.togglePlay() },
        ) {
            Artwork(Art.Video(track.videoId), side)
        }
    }
}

@Composable
private fun Header(track: Track) {
    Column(Modifier.fillMaxWidth().padding(horizontal = 32.dp)) {
        Text(
            track.title, fontSize = 24.sp, fontWeight = FontWeight.Bold,
            maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.basicMarquee(),
        )
        // The artist opens the artist's page.
        Text(
            track.artist, fontSize = 17.sp, maxLines = 1, overflow = TextOverflow.Ellipsis, color = YT.secondary,
            modifier = Modifier.clickable { Router.openArtist(track) },
        )
        Status()
    }
}

/** 👍 | 👎, Save, Download, Share — YouTube Music's pill row. */
@Composable
private fun Actions(track: Track, onLyrics: () -> Unit) {
    val app = App.instance
    val library = app.library
    val context = LocalContext.current
    var saving by remember { mutableStateOf(false) }
    Row(
        Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()).padding(horizontal = 20.dp),
        horizontalArrangement = Arrangement.spacedBy(8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        val liked = library.isLiked(track)
        Row(Modifier.height(36.dp).clip(CircleShape).background(PILL), verticalAlignment = Alignment.CenterVertically) {
            Box(Modifier.fillMaxHeight().clickable { library.toggleLike(track) }.padding(horizontal = 14.dp), contentAlignment = Alignment.Center) {
                Icon(if (liked) Icons.Filled.ThumbUp else Icons.Outlined.ThumbUp, stringResource(R.string.like), Modifier.size(20.dp))
            }
            Box(Modifier.width(1.dp).height(18.dp).background(Color.White.copy(alpha = 0.25f)))
            Box(Modifier.fillMaxHeight().clickable { app.player.dislike(track) }.padding(horizontal = 14.dp), contentAlignment = Alignment.Center) {
                Icon(Icons.Outlined.ThumbDown, stringResource(R.string.dislike), Modifier.size(20.dp))
            }
        }
        Pill(Icons.Default.FormatQuote, stringResource(R.string.lyrics), onClick = onLyrics)
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
        Pill(Icons.Default.Sensors, stringResource(R.string.mix)) { app.player.startRadio(track) }
    }
    if (saving) PlaylistChooser(track) { saving = false }
}

@Composable
private fun Pill(icon: androidx.compose.ui.graphics.vector.ImageVector, label: String, onClick: () -> Unit) {
    Row(
        Modifier.height(36.dp).clip(CircleShape).background(PILL).clickable(onClick = onClick).padding(horizontal = 14.dp),
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

/**
 * YouTube Music's scrubber on Android: a thin line with a dot, the times
 * under it (its own composable so ticks don't redraw the whole screen).
 */
@Composable
private fun Scrubber() {
    val player = App.instance.player
    var dragging by remember { mutableStateOf<Double?>(null) }
    val duration = maxOf(player.duration, 1.0)
    val shown = dragging ?: player.position
    Column(Modifier.padding(horizontal = 32.dp)) {
        BoxWithConstraints(
            Modifier
                .fillMaxWidth()
                .height(24.dp)
                .pointerInput(duration) {
                    detectTapGestures { player.seek(duration * (it.x / size.width).coerceIn(0f, 1f)) }
                }
                .pointerInput(duration) {
                    detectHorizontalDragGestures(
                        onDragStart = { dragging = duration * (it.x / size.width).coerceIn(0f, 1f) },
                        onDragEnd = {
                            dragging?.let(player::seek)
                            dragging = null
                        },
                        onDragCancel = { dragging = null },
                    ) { change, _ -> dragging = duration * (change.position.x / size.width).coerceIn(0f, 1f) }
                },
            contentAlignment = Alignment.CenterStart,
        ) {
            val fraction = (shown / duration).toFloat().coerceIn(0f, 1f)
            val dot = if (dragging != null) 16.dp else 12.dp
            Box(Modifier.fillMaxWidth().height(2.dp).background(Color.White.copy(alpha = 0.25f)))
            Box(Modifier.fillMaxWidth(fraction).height(2.dp).background(Color.White))
            Box(Modifier.offset(x = maxWidth * fraction - dot / 2).size(dot).clip(CircleShape).background(Color.White))
        }
        Row {
            Text(formatTime(shown), fontSize = 12.sp, color = YT.secondary)
            Spacer(Modifier.weight(1f))
            Text(formatTime(player.duration), fontSize = 12.sp, color = YT.secondary)
        }
    }
}

@Composable
private fun Controls() {
    val player = App.instance.player
    Row(
        Modifier.fillMaxWidth().padding(horizontal = 20.dp),
        horizontalArrangement = Arrangement.SpaceBetween, verticalAlignment = Alignment.CenterVertically,
    ) {
        IconButton(onClick = { player.shuffleUpcoming() }) { Icon(Icons.Default.Shuffle, stringResource(R.string.shuffle)) }
        IconButton(onClick = { player.previous() }) {
            Icon(Icons.Default.SkipPrevious, stringResource(R.string.previous), Modifier.size(40.dp))
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
        IconButton(onClick = { player.next() }) {
            Icon(Icons.Default.SkipNext, stringResource(R.string.next), Modifier.size(40.dp))
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

/** Sleep timer: pause after a while, or when the song ends. */
@Composable
private fun SleepDialog(onDismiss: () -> Unit) {
    val player = App.instance.player
    val on = player.sleepUntil != null || player.sleepAtEndOfSong
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(stringResource(R.string.sleep_timer)) },
        text = {
            Column {
                @Composable
                fun option(label: String, run: () -> Unit) = Text(
                    label,
                    Modifier.fillMaxWidth().clickable {
                        run()
                        onDismiss()
                    }.padding(vertical = 12.dp),
                )
                listOf(15, 30, 60).forEach { minutes -> option(stringResource(R.string.minutes, minutes)) { player.setSleep(minutes) } }
                option(stringResource(R.string.end_of_song)) { player.setSleep(null) }
                if (on) option(stringResource(R.string.turn_off)) { player.cancelSleep() }
            }
        },
        confirmButton = {},
        dismissButton = { TextButton(onClick = onDismiss) { Text(stringResource(R.string.cancel)) } },
    )
}

/** Karaoke lyrics: big bold lines, the sung one lighting up word by word; a tap seeks to a line. */
@Composable
private fun LyricsPane(track: Track) {
    val app = App.instance
    val player = app.player
    val lyrics = rememberLoad(track.videoId) {
        val duration = track.durationSecs?.toDouble() ?: player.duration.takeIf { it > 0 }
        // Kept on the phone (a download's, or played lately): offline too.
        dev.yarek.music.KeptLyrics.find(track, duration, app.library.downloadedIds) ?: NoLyrics
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

/**
 * YouTube Music's queue: "Save" at the top, the playing song highlighted,
 * then what's next — drag ≡ to reorder, swipe a song away to remove it.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun QueuePane() {
    val player = App.instance.player
    val current = player.current
    val upNext = player.upNext
    var naming by remember { mutableStateOf(false) }
    val rowHeight = with(androidx.compose.ui.platform.LocalDensity.current) { 64.dp.toPx() }
    LazyColumn(Modifier.fillMaxSize()) {
        item {
            Row(Modifier.fillMaxWidth().padding(start = 16.dp, end = 16.dp, top = 12.dp, bottom = 8.dp), verticalAlignment = Alignment.CenterVertically) {
                Column(Modifier.weight(1f)) {
                    Text(stringResource(R.string.up_next), fontSize = 14.sp, color = YT.secondary)
                    Text(songsCount(upNext.size + 1), fontSize = 15.sp, fontWeight = FontWeight.Bold)
                }
                // The whole queue (what played, the song, what's next) as a playlist.
                Row(
                    Modifier.clip(CircleShape).background(Color.White.copy(alpha = 0.1f)).clickable { naming = true }
                        .padding(horizontal = 16.dp, vertical = 8.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Icon(Icons.AutoMirrored.Filled.PlaylistAdd, null, Modifier.size(20.dp))
                    Spacer(Modifier.width(8.dp))
                    Text(stringResource(R.string.save), fontWeight = FontWeight.Medium)
                }
            }
        }
        if (current != null) item { QueueRow(current, highlighted = true, handle = null) {} }
        if (upNext.isEmpty()) {
            item {
                Message(
                    Icons.AutoMirrored.Filled.QueueMusic, stringResource(R.string.queue_empty),
                    if (player.autoplay) stringResource(R.string.similar_will_follow) else null,
                )
            }
        }
        // Keys follow the songs (the nth copy of a song keeps its key), so a
        // drag survives the moves it makes.
        val seen = HashMap<String, Int>()
        val keys = upNext.map { t -> "${t.videoId}#${seen.merge(t.videoId, 1, Int::plus)}" }
        itemsIndexed(upNext, key = { i, _ -> keys[i] }) { i, track ->
            val index by rememberUpdatedState(i)
            val dismiss = rememberSwipeToDismissBoxState(confirmValueChange = {
                if (it != SwipeToDismissBoxValue.Settled) player.removeUpcoming(index)
                it != SwipeToDismissBoxValue.Settled
            })
            SwipeToDismissBox(dismiss, backgroundContent = {}) {
                var drag by remember { mutableFloatStateOf(0f) }
                var at by remember { mutableIntStateOf(index) }
                QueueRow(
                    track, highlighted = false,
                    modifier = Modifier.graphicsLayer { translationY = drag },
                    handle = Modifier.pointerInput(Unit) {
                        detectDragGestures(
                            onDragStart = { at = index; drag = 0f },
                            onDragEnd = { drag = 0f },
                            onDragCancel = { drag = 0f },
                        ) { change, amount ->
                            change.consume()
                            drag += amount.y
                            // A row's height dragged: swap with the neighbour.
                            val target = at + (drag / rowHeight).toInt()
                            if (target != at && target in upNext.indices) {
                                player.moveUpcoming(at, target)
                                drag -= (target - at) * rowHeight
                                at = target
                            }
                        }
                    },
                ) { player.jump(player.index + 1 + index) }
            }
        }
    }
    if (naming) SaveQueueDialog { naming = false }
}

@Composable
private fun QueueRow(track: Track, highlighted: Boolean, modifier: Modifier = Modifier, handle: Modifier?, onClick: () -> Unit) {
    Row(
        modifier.fillMaxWidth().height(64.dp)
            .background(if (highlighted) Color.White.copy(alpha = 0.12f) else SHEET)
            .clickable(onClick = onClick).padding(start = 16.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Artwork(Art.Video(track.videoId), 48.dp)
        Spacer(Modifier.width(16.dp))
        Column(Modifier.weight(1f)) {
            Text(track.title, maxLines = 1, overflow = TextOverflow.Ellipsis, fontSize = 16.sp)
            Row(verticalAlignment = Alignment.CenterVertically) {
                if (track.explicit) {
                    ExplicitBadge()
                    Spacer(Modifier.width(5.dp))
                }
                val duration = track.durationSecs?.let { " • " + formatTime(it.toDouble()) }.orEmpty()
                Text(track.artist + duration, maxLines = 1, overflow = TextOverflow.Ellipsis, fontSize = 14.sp, color = YT.secondary)
            }
        }
        if (handle != null) {
            Box(handle.size(56.dp), contentAlignment = Alignment.Center) {
                Icon(Icons.Default.DragHandle, stringResource(R.string.reorder), tint = Color.White)
            }
        } else {
            Spacer(Modifier.width(56.dp))
        }
    }
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

/**
 * Above the tab bar, as YouTube Music draws it: the song and play / pause; a
 * tap opens the player, a swipe sideways skips.
 */
@Composable
fun MiniPlayer() {
    val player = App.instance.player
    val track = player.current ?: return
    // Swipe sideways: next / previous song (the row follows the finger);
    // swipe up: open the player — as in YouTube Music.
    var swipe by remember { mutableStateOf(androidx.compose.ui.geometry.Offset.Zero) }
    Column(
        Modifier.fillMaxWidth().background(YT.surface).clickable { player.showNowPlaying = true }
            .pointerInput(Unit) {
                detectDragGestures(
                    onDragEnd = {
                        val (x, y) = swipe
                        when {
                            -y > 60 && -y > kotlin.math.abs(x) -> player.showNowPlaying = true
                            x < -120 -> player.next()
                            x > 120 -> player.previous(always = true)
                        }
                        swipe = androidx.compose.ui.geometry.Offset.Zero
                    },
                    onDragCancel = { swipe = androidx.compose.ui.geometry.Offset.Zero },
                ) { change, amount ->
                    change.consume()
                    swipe += amount
                }
            },
    ) {
        // Progress line along the top.
        val fraction = if (player.duration > 0) (player.position / player.duration).toFloat().coerceIn(0f, 1f) else 0f
        Box(Modifier.fillMaxWidth().height(2.dp).background(Color.White.copy(alpha = 0.15f))) {
            Box(Modifier.fillMaxWidth(fraction).height(2.dp).background(Color.White))
        }
        Row(
            Modifier.height(64.dp).graphicsLayer { if (kotlin.math.abs(swipe.x) > kotlin.math.abs(swipe.y)) translationX = swipe.x }
                .padding(start = 16.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Artwork(Art.Video(track.videoId), 48.dp)
            Spacer(Modifier.width(16.dp))
            Column(Modifier.weight(1f)) {
                Text(track.title, fontWeight = FontWeight.Medium, fontSize = 16.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
                if (player.error != null || player.waitingForNetwork || (player.slow && player.isPlaying)) {
                    Status(compact = true)
                } else {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        if (track.explicit) {
                            ExplicitBadge()
                            Spacer(Modifier.width(5.dp))
                        }
                        Text(track.artist, fontSize = 16.sp, color = YT.secondary, maxLines = 1, overflow = TextOverflow.Ellipsis)
                    }
                }
            }
            IconButton(onClick = { player.togglePlay() }, Modifier.size(56.dp)) {
                if (player.isBuffering && player.isPlaying) {
                    CircularProgressIndicator(Modifier.size(24.dp), strokeWidth = 2.dp, color = Color.White)
                } else {
                    Icon(
                        if (player.isPlaying) Icons.Default.Pause else Icons.Default.PlayArrow,
                        stringResource(if (player.isPlaying) R.string.pause else R.string.play),
                        Modifier.size(34.dp),
                    )
                }
            }
        }
    }
}

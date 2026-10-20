package dev.ytmplayer.android

import android.content.Context
import android.content.Intent
import android.net.Uri
import androidx.annotation.OptIn
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableDoubleStateOf
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.media3.common.AudioAttributes
import androidx.media3.common.C
import androidx.media3.common.MediaItem
import androidx.media3.common.MediaMetadata
import androidx.media3.common.MimeTypes
import androidx.media3.common.PlaybackException
import androidx.media3.common.Player as Media3Player
import androidx.media3.common.util.UnstableApi
import androidx.media3.datasource.DefaultDataSource
import androidx.media3.datasource.DefaultHttpDataSource
import androidx.media3.datasource.HttpDataSource
import androidx.media3.datasource.ResolvingDataSource
import androidx.media3.exoplayer.DefaultLoadControl
import androidx.media3.exoplayer.ExoPlayer
import androidx.media3.exoplayer.source.DefaultMediaSourceFactory
import androidx.media3.exoplayer.upstream.DefaultLoadErrorHandlingPolicy
import androidx.media3.exoplayer.upstream.LoadErrorHandlingPolicy
import dev.ytmplayer.core.Catalog
import dev.ytmplayer.core.Resolver
import dev.ytmplayer.core.Stream
import dev.ytmplayer.core.Track
import dev.ytmplayer.core.arr
import dev.ytmplayer.core.isNetworkFailure
import dev.ytmplayer.core.objects
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import org.json.JSONArray
import org.json.JSONObject
import java.io.File
import java.io.IOException
import java.util.concurrent.ConcurrentHashMap
import kotlin.math.pow

enum class RepeatMode {
    Off, All, One;

    val next: RepeatMode get() = entries[(ordinal + 1) % entries.size]
}

/** The song's stream URL couldn't be found (YouTube said no, or no network). */
class ResolveFailed(cause: Throwable) : IOException(cause.message, cause)

/** The song only plays as HLS (age-restricted, signed in): its item must be an HLS one. */
class NeedsHls(val videoId: String) : IOException("HLS only")

/**
 * Queue + playback on ExoPlayer. The whole queue sits in ExoPlayer's
 * playlist as `ytm://<video id>` items; a ResolvingDataSource turns each
 * into its googlevideo URL (the Resolver, on ExoPlayer's loader thread)
 * when ExoPlayer starts loading it, so the next song preloads and plays
 * gaplessly, and the notification's next / previous work by themselves.
 * Downloaded songs are plain files. Crossfade lets a second, short-lived
 * player finish the old song while the main one starts the next.
 */
@OptIn(UnstableApi::class)
class Player(
    private val context: Context,
    private val library: Library,
    private val resolver: Resolver,
    private val catalog: Catalog,
    private val network: Network,
    private val settings: Settings,
) {
    var queue by mutableStateOf<List<Track>>(emptyList())
        private set
    var index by mutableIntStateOf(0)
        private set

    /** Playing, or about to (the intent: true while the song resolves and buffers). */
    var isPlaying by mutableStateOf(false)
        private set

    /** Resolving the URL or waiting for data. */
    var isBuffering by mutableStateOf(false)
        private set

    /** Buffering for a while: the connection is slow. */
    var slow by mutableStateOf(false)
        private set
    var position by mutableDoubleStateOf(0.0)
        private set
    var duration by mutableDoubleStateOf(0.0)
        private set
    var error by mutableStateOf<String?>(null)

    /** Playback stopped because the connection went; it resumes when it comes back. */
    var waitingForNetwork by mutableStateOf(false)
        private set
    var showNowPlaying by mutableStateOf(false)

    var repeatMode by mutableStateOf(RepeatMode.Off)
        private set
    var autoplay by mutableStateOf(settings.autoplay)
        private set
    var crossfade by mutableIntStateOf(settings.crossfade)
        private set
    var normalize by mutableStateOf(settings.normalize)
        private set

    /** Sleep timer: pause at this moment (epoch ms), or at the end of the song. */
    var sleepUntil by mutableStateOf<Long?>(null)
        private set
    var sleepAtEndOfSong by mutableStateOf(false)
        private set

    val current: Track? get() = queue.getOrNull(index)
    val upNext: List<Track> get() = queue.drop(index + 1)

    /** ExoPlayer's own clock (karaoke lyrics read it ten times a second). */
    val exactPosition: Double get() = exo.currentPosition / 1000.0

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)

    /** Resolved streams by video id (their loudness sets the volume). */
    private val streams = ConcurrentHashMap<String, Stream>()

    /** Songs that play only as HLS. */
    private val hlsOnly = ConcurrentHashMap.newKeySet<String>()

    /** Songs whose URL was already refreshed once after a failure. */
    private val retried = HashSet<String>()
    private var failuresInARow = 0
    private var networkRetries = 0
    private var radioSeed: String? = null
    private var ticker: Job? = null
    private var bufferingSince = 0L
    private var retryJob: Job? = null

    /** Last session's queue going back in: not a song being played. */
    private var restoring = false

    val exo: ExoPlayer = build(focus = true)

    /** The old song during a crossfade. */
    private var ghost: ExoPlayer? = null
    private var ghostStartMs = 0L
    private var fadeJob: Job? = null

    init {
        exo.addListener(Events())
        restoreQueue()
        scope.launch {
            network.online.collect { online ->
                if (online && waitingForNetwork) {
                    waitingForNetwork = false
                    error = null
                    networkRetries = 0
                    exo.prepare()
                }
            }
        }
    }

    private fun build(focus: Boolean): ExoPlayer {
        val http = DefaultHttpDataSource.Factory()
            .setConnectTimeoutMs(dev.ytmplayer.core.Http.CONNECT_TIMEOUT_MS)
            .setReadTimeoutMs(dev.ytmplayer.core.Http.READ_TIMEOUT_MS)
            .setAllowCrossProtocolRedirects(true)
        // Ranges, not one long read: googlevideo throttles those (ChunkedHttp.kt).
        val resolving = ResolvingDataSource.Factory(DefaultDataSource.Factory(context, ChunkedHttpDataSource.Factory(http))) { spec ->
            val scheme = spec.uri.scheme
            if (scheme != "ytm" && scheme != "ytmhls") return@Factory spec
            val id = spec.uri.host ?: throw ResolveFailed(IllegalArgumentException("no video id"))
            val stream = try {
                runBlocking { resolver.stream(id) }
            } catch (e: Exception) {
                throw ResolveFailed(e)
            }
            streams[id] = stream
            if (scheme == "ytm" && stream.hls) {
                hlsOnly += id
                throw NeedsHls(id)
            }
            spec.withUri(Uri.parse(stream.url))
        }
        // Enough to ride out a weak signal, not the whole song in memory.
        val load = DefaultLoadControl.Builder().setBufferDurationsMs(15_000, 30_000, 1_500, 3_000).build()
        val player = ExoPlayer.Builder(context)
            .setMediaSourceFactory(DefaultMediaSourceFactory(resolving).setLoadErrorHandlingPolicy(ErrorPolicy()))
            .setLoadControl(load)
            .setAudioAttributes(
                AudioAttributes.Builder().setUsage(C.USAGE_MEDIA).setContentType(C.AUDIO_CONTENT_TYPE_MUSIC).build(),
                focus,
            )
            .setHandleAudioBecomingNoisy(focus)
            .setWakeMode(if (focus) C.WAKE_MODE_NETWORK else C.WAKE_MODE_NONE)
            .build()
        // Muxed videos (age-restricted songs): decode only the audio.
        player.trackSelectionParameters = player.trackSelectionParameters.buildUpon()
            .setTrackTypeDisabled(C.TRACK_TYPE_VIDEO, true).build()
        return player
    }

    /**
     * No retries for what can't get better by retrying (YouTube refused the
     * song, an expired URL, no network at all); a few for a flaky connection.
     */
    private inner class ErrorPolicy : DefaultLoadErrorHandlingPolicy() {
        override fun getRetryDelayMsFor(info: LoadErrorHandlingPolicy.LoadErrorInfo): Long {
            val e = info.exception
            if (e is NeedsHls) return C.TIME_UNSET
            if (e is ResolveFailed && !(e.cause?.isNetworkFailure() ?: false)) return C.TIME_UNSET
            if (e is HttpDataSource.InvalidResponseCodeException && e.responseCode in listOf(403, 410)) return C.TIME_UNSET
            if (!network.online.value) return C.TIME_UNSET
            return if (info.errorCount <= 4) 1_500L * info.errorCount else C.TIME_UNSET
        }

        override fun getMinimumLoadableRetryCount(dataType: Int): Int = 4
    }

    // Commands

    fun play(tracks: List<Track>, start: Int = 0) {
        if (start !in tracks.indices) return
        finishFade()
        queue = tracks
        index = start
        radioSeed = null
        retried.clear()
        failuresInARow = 0
        error = null
        exo.setMediaItems(tracks.map(::item), start, 0)
        startPlayback()
        saveQueue()
        topUpRadio()
    }

    /** The song, then its radio. */
    fun startRadio(track: Track) {
        play(listOf(track))
        appendRadio(track)
    }

    fun playNext(track: Track) {
        if (current == null) return play(listOf(track))
        queue = queue.toMutableList().apply { add(index + 1, track) }
        exo.addMediaItem(index + 1, item(track))
        saveQueue()
    }

    fun addToQueue(track: Track) {
        if (current == null) return play(listOf(track))
        queue = queue + track
        exo.addMediaItem(item(track))
        saveQueue()
    }

    fun jump(to: Int) {
        if (to !in queue.indices) return
        finishFade()
        exo.seekTo(to, 0)
        startPlayback()
    }

    fun removeUpcoming(offset: Int) {
        val at = index + 1 + offset
        if (at !in queue.indices) return
        queue = queue.filterIndexed { i, _ -> i != at }
        exo.removeMediaItem(at)
        saveQueue()
    }

    fun moveUpcoming(from: Int, to: Int) {
        val a = index + 1 + from
        val b = index + 1 + to
        if (a !in queue.indices || b !in queue.indices || a == b) return
        queue = queue.toMutableList().apply { add(b, removeAt(a)) }
        exo.moveMediaItem(a, b)
        saveQueue()
    }

    fun shuffleUpcoming() {
        if (index + 1 >= queue.size) return
        val rest = queue.drop(index + 1).shuffled()
        queue = queue.take(index + 1) + rest
        exo.replaceMediaItems(index + 1, exo.mediaItemCount, rest.map(::item))
        saveQueue()
    }

    fun togglePlay() {
        finishFade()
        when {
            exo.playbackState == Media3Player.STATE_IDLE || exo.playbackState == Media3Player.STATE_ENDED -> {
                if (exo.playbackState == Media3Player.STATE_ENDED) exo.seekTo(index, 0)
                startPlayback()
            }
            exo.playWhenReady -> exo.pause()
            else -> exo.play()
        }
    }

    fun pause() {
        finishFade()
        exo.pause()
    }

    fun next() {
        finishFade()
        if (exo.hasNextMediaItem()) {
            exo.seekToNextMediaItem()
            if (exo.playbackState == Media3Player.STATE_IDLE) startPlayback()
        }
    }

    fun previous() {
        finishFade()
        if (exo.currentPosition > 3_000 || !exo.hasPreviousMediaItem()) {
            exo.seekTo(0)
        } else {
            exo.seekToPreviousMediaItem()
        }
        if (exo.playbackState == Media3Player.STATE_IDLE) startPlayback()
    }

    fun seek(seconds: Double) {
        finishFade()
        exo.seekTo((seconds * 1000).toLong())
        position = seconds
    }

    fun cycleRepeat() {
        repeatMode = repeatMode.next
        exo.repeatMode = when (repeatMode) {
            RepeatMode.Off -> Media3Player.REPEAT_MODE_OFF
            RepeatMode.All -> Media3Player.REPEAT_MODE_ALL
            RepeatMode.One -> Media3Player.REPEAT_MODE_ONE
        }
    }

    fun changeAutoplay(on: Boolean) {
        autoplay = on
        settings.autoplay = on
        if (on) topUpRadio()
    }

    fun changeCrossfade(seconds: Int) {
        crossfade = seconds
        settings.crossfade = seconds
    }

    fun changeNormalize(on: Boolean) {
        normalize = on
        settings.normalize = on
        applyGain()
    }

    /** Pauses in `minutes`, or at the end of this song when null. */
    fun setSleep(minutes: Int?) {
        if (minutes != null) {
            sleepUntil = System.currentTimeMillis() + minutes * 60_000L
            sleepAtEndOfSong = false
        } else {
            sleepUntil = null
            sleepAtEndOfSong = true
        }
    }

    fun cancelSleep() {
        sleepUntil = null
        sleepAtEndOfSong = false
    }

    /** The clip is on screen: the audio waits. */
    fun pauseForVideo(): Double {
        finishFade()
        val at = exactPosition
        exo.pause()
        return at
    }

    /** Back from the clip: the song carries on where the video stopped. */
    fun resumeAfterVideo(at: Double?) {
        at?.let { exo.seekTo((it * 1000).toLong()) }
        if (exo.playbackState == Media3Player.STATE_IDLE) exo.prepare()
        exo.play()
    }

    /** Dislikes the song (YouTube learns from it) and moves on if it plays. */
    fun dislike(track: Track) {
        library.dislike(track)
        if (current?.videoId == track.videoId) next()
    }

    // Internals

    private fun startPlayback() {
        error = null
        waitingForNetwork = false
        exo.prepare()
        exo.play()
    }

    /**
     * The service puts the session in the notification and keeps the
     * process alive in the background. Started whenever playback starts;
     * from the background that is refused, but then it already runs.
     */
    private fun ensureService() {
        runCatching { context.startService(Intent(context, PlaybackService::class.java)) }
    }

    private fun item(track: Track): MediaItem {
        val builder = MediaItem.Builder()
            .setMediaId(track.videoId)
            .setMediaMetadata(
                MediaMetadata.Builder()
                    .setTitle(track.title)
                    .setArtist(track.artist)
                    .setArtworkUri(Uri.parse(track.largeThumbnailUrl))
                    .build(),
            )
        val downloaded = library.download(track.videoId)
        when {
            downloaded != null -> builder.setUri(Uri.fromFile(downloaded.first))
            track.videoId in hlsOnly -> builder.setUri("ytmhls://${track.videoId}").setMimeType(MimeTypes.APPLICATION_M3U8)
            else -> builder.setUri("ytm://${track.videoId}")
        }
        return builder.build()
    }

    private fun gain(videoId: String): Float {
        if (!normalize) return 1f
        val db = streams[videoId]?.loudnessDb ?: library.download(videoId)?.second?.loudnessDb ?: return 1f
        return if (db > 0) 10.0.pow(-db / 20).toFloat() else 1f
    }

    private fun applyGain() {
        if (fadeJob == null) current?.let { exo.volume = gain(it.videoId) }
    }

    /** A song began: history, more radio. */
    private fun started() {
        val track = current ?: return
        failuresInARow = 0
        networkRetries = 0
        error = null
        library.played(track)
        duration = track.durationSecs?.toDouble() ?: 0.0
        applyGain()
        topUpRadio()
        saveQueue()
    }

    private fun topUpRadio() {
        if (autoplay && repeatMode == RepeatMode.Off && index >= queue.size - 2) queue.lastOrNull()?.let(::appendRadio)
    }

    private fun appendRadio(track: Track) {
        if (radioSeed == track.videoId) return
        radioSeed = track.videoId
        scope.launch {
            val radio = runCatching { catalog.radio(track.videoId) }.getOrNull() ?: run {
                // Offline: try again with the next song.
                radioSeed = null
                return@launch
            }
            val have = queue.map { it.videoId }.toSet()
            val more = radio.filter { it.videoId !in have }
            if (more.isEmpty()) return@launch
            queue = queue + more
            exo.addMediaItems(more.map(::item))
            saveQueue()
        }
    }

    private inner class Events : Media3Player.Listener {
        override fun onMediaItemTransition(mediaItem: MediaItem?, reason: Int) {
            index = exo.currentMediaItemIndex
            position = exo.currentPosition / 1000.0
            if (reason == Media3Player.MEDIA_ITEM_TRANSITION_REASON_AUTO && sleepAtEndOfSong) {
                // The next song already started (gapless): stop it at its start.
                sleepAtEndOfSong = false
                exo.pause()
                exo.seekTo(0)
            }
            if (mediaItem != null && !restoring) started()
        }

        override fun onIsPlayingChanged(playing: Boolean) {
            if (playing) startTicker()
        }

        override fun onPlayWhenReadyChanged(playWhenReady: Boolean, reason: Int) {
            isPlaying = playWhenReady
            if (playWhenReady) {
                ensureService()
                startTicker()
            }
        }

        override fun onPlaybackStateChanged(state: Int) {
            val buffering = state == Media3Player.STATE_BUFFERING
            if (buffering && !isBuffering) bufferingSince = System.currentTimeMillis()
            isBuffering = buffering
            if (!buffering) slow = false
            if (state == Media3Player.STATE_READY) {
                val d = exo.duration
                if (d != C.TIME_UNSET) duration = d / 1000.0
                applyGain()
            }
            if (state == Media3Player.STATE_ENDED) {
                position = 0.0
                isPlaying = false
            }
        }

        override fun onPlayerError(e: PlaybackException) = handleError(e)
    }

    private fun handleError(e: PlaybackException) {
        val track = current ?: return
        val hls = generateSequence<Throwable>(e) { it.cause }.filterIsInstance<NeedsHls>().firstOrNull()
        if (hls != null) {
            // Same place in the queue, as an HLS item now.
            exo.replaceMediaItem(index, item(track))
            exo.prepare()
            return
        }
        val http = generateSequence<Throwable>(e) { it.cause }.filterIsInstance<HttpDataSource.InvalidResponseCodeException>().firstOrNull()
        if (http != null && http.responseCode in listOf(403, 410) && retried.add(track.videoId)) {
            // An expired URL, or the network moved to another IP: resolve
            // once more and continue where it stopped.
            scope.launch {
                resolver.forget(track.videoId)
                exo.prepare()
            }
            return
        }
        val online = network.online.value
        val cause = generateSequence<Throwable>(e) { it.cause }.firstOrNull { it is ResolveFailed }?.cause ?: e.cause ?: e
        val networkProblem = !online || e.errorCode in NETWORK_ERRORS || cause.isNetworkFailure()
        error = context.getString(R.string.track_error, track.title, cause.describe(context, online))
        if (networkProblem) {
            // Don't run through the queue while the connection is down:
            // wait here and carry on when it comes back.
            if (!online) {
                waitingForNetwork = true
                error = null
                return
            }
            if (networkRetries++ < 3) {
                retryJob?.cancel()
                retryJob = scope.launch {
                    delay(3_000L * networkRetries)
                    if (exo.playbackState == Media3Player.STATE_IDLE) exo.prepare()
                }
            }
            return
        }
        // Skip what can't play (age-restricted, removed), but don't run
        // through a whole queue of failures.
        failuresInARow++
        if (failuresInARow < 3 && exo.hasNextMediaItem()) {
            val target = index
            scope.launch {
                delay(2_000)
                if (index == target && exo.playbackState == Media3Player.STATE_IDLE) {
                    exo.seekToNextMediaItem()
                    exo.prepare()
                }
            }
        }
    }

    private fun startTicker() {
        if (ticker?.isActive == true) return
        ticker = scope.launch {
            while (isActive && (exo.playWhenReady || fadeJob != null)) {
                tick()
                delay(500)
            }
            tick()
        }
    }

    private fun tick() {
        position = exo.currentPosition / 1000.0
        val d = exo.duration
        if (d != C.TIME_UNSET && d > 0) duration = d / 1000.0
        if (isBuffering && System.currentTimeMillis() - bufferingSince > SLOW_AFTER_MS) slow = true
        sleepUntil?.let {
            if (System.currentTimeMillis() >= it) {
                sleepUntil = null
                pause()
            }
        }
        crossfadeTick()
    }

    // Crossfade

    /**
     * `crossfade` seconds before the end: a second player picks up the old
     * song a moment ahead (`LEAD_MS`, time to buffer), and once the main
     * player gets there it moves on to the next song while the volumes
     * swap over. The main player (and the media session) never changes.
     */
    private fun crossfadeTick() {
        if (crossfade <= 0 || fadeJob != null || !exo.isPlaying || repeatMode == RepeatMode.One || !exo.hasNextMediaItem()) return
        val track = current ?: return
        val length = exo.duration.takeIf { it != C.TIME_UNSET } ?: return
        val now = exo.currentPosition
        val left = length - now
        val g = ghost
        if (g == null) {
            if (left > crossfade * 1000L + LEAD_MS || left < LEAD_MS + 500) return
            ghost = build(focus = false).also { p ->
                ghostStartMs = now + LEAD_MS
                p.setMediaItem(item(track), ghostStartMs)
                p.volume = 0f
                p.prepare()
            }
            return
        }
        if (now < ghostStartMs) return
        if (g.playbackState != Media3Player.STATE_READY) {
            // Too slow to buffer: no fade this time.
            dropGhost()
            return
        }
        val from = gain(track.videoId)
        g.volume = from
        g.play()
        exo.volume = 0f
        exo.seekToNextMediaItem()
        val span = minOf(crossfade * 1000L, length - ghostStartMs).coerceAtLeast(500)
        fadeJob = scope.launch {
            val start = System.currentTimeMillis()
            while (isActive) {
                val t = ((System.currentTimeMillis() - start).toFloat() / span).coerceIn(0f, 1f)
                g.volume = from * (1 - t)
                exo.volume = (current?.let { gain(it.videoId) } ?: 1f) * t
                if (t >= 1f) break
                delay(50)
            }
            fadeJob = null
            dropGhost()
            applyGain()
        }
    }

    /** Something else happened mid-fade (pause, seek, another song): end it now. */
    private fun finishFade() {
        val job = fadeJob ?: run {
            dropGhost()
            return
        }
        job.cancel()
        fadeJob = null
        dropGhost()
        applyGain()
    }

    private fun dropGhost() {
        ghost?.release()
        ghost = null
    }

    // Persistence

    private val queueFile get() = File(context.filesDir, "queue.json")

    private fun saveQueue() {
        val text = JSONObject()
            .put("tracks", JSONArray(queue.take(500).map { it.toJson() }))
            .put("index", index)
            .toString()
        scope.launch(Dispatchers.IO) { runCatching { queueFile.writeText(text) } }
    }

    /** Last session's queue, paused at its song (resolved only when played). */
    private fun restoreQueue() {
        val saved = runCatching { JSONObject(queueFile.readText()) }.getOrNull() ?: return
        val tracks = saved.arr("tracks")?.objects()?.mapNotNull(Track::fromJson).orEmpty()
        val at = saved.optInt("index")
        if (at !in tracks.indices) return
        queue = tracks
        index = at
        duration = tracks[at].durationSecs?.toDouble() ?: 0.0
        restoring = true
        exo.setMediaItems(tracks.map(::item), at, 0)
        restoring = false
    }

    private companion object {
        const val LEAD_MS = 1_500L
        const val SLOW_AFTER_MS = 4_000L
        val NETWORK_ERRORS = setOf(
            PlaybackException.ERROR_CODE_IO_NETWORK_CONNECTION_FAILED,
            PlaybackException.ERROR_CODE_IO_NETWORK_CONNECTION_TIMEOUT,
            PlaybackException.ERROR_CODE_IO_UNSPECIFIED,
        )
    }
}

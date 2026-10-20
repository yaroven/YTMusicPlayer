package dev.yarek.music

import android.content.Context
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import dev.yarek.core.Catalog
import dev.yarek.core.Http
import dev.yarek.core.ItemKind
import dev.yarek.core.Page
import dev.yarek.core.Rating
import dev.yarek.core.Resolver
import dev.yarek.core.Track
import dev.yarek.core.YTMError
import dev.yarek.core.arr
import dev.yarek.core.objects
import dev.yarek.core.str
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.json.JSONArray
import org.json.JSONObject
import java.io.File
import java.io.RandomAccessFile
import java.util.UUID

data class LocalPlaylist(
    val id: String = UUID.randomUUID().toString(),
    val name: String,
    val tracks: List<Track> = emptyList(),
    /** YouTube's id (`PL…`) when this playlist is the account's own. */
    val remoteId: String? = null,
    /** Per track, the id a removal needs (same order as `tracks`). */
    val setVideoIds: List<String> = emptyList(),
)

/** A saved album, artist or playlist page. */
data class SavedPage(
    val id: String,
    val kind: ItemKind,
    val title: String,
    val subtitle: String,
    val thumbnail: String?,
    /** Came from the account (replaced by every sync) rather than saved here. */
    val synced: Boolean = false,
)

/** A song saved on the phone for offline play. */
data class Downloaded(
    val track: Track,
    val file: String,
    val bytes: Long,
    val duration: Double?,
    val loudnessDb: Double?,
    /** The copy in the phone's Music folder (a MediaStore URI), if any. */
    val exported: String? = null,
)

sealed interface LocalList {
    object Liked : LocalList
    object History : LocalList
    object Downloads : LocalList
    data class Playlist(val id: String) : LocalList
}

/**
 * The library: liked songs, playlists and saved pages of the signed-in
 * account (synced from YouTube Music and written back), plus history and
 * downloads. One small JSON file — a port of the iPhone app's Library.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class Library(private val context: Context, private val catalog: Catalog, private val resolver: Resolver) {
    var liked by mutableStateOf<List<Track>>(emptyList())
        private set
    var history by mutableStateOf<List<Track>>(emptyList())
        private set

    /** How often each song was played (by video id); outlives the capped history. */
    var plays by mutableStateOf<Map<String, Int>>(emptyMap())
        private set
    var playlists by mutableStateOf<List<LocalPlaylist>>(emptyList())
        private set
    var saved by mutableStateOf<List<SavedPage>>(emptyList())
        private set

    /** Songs on the phone, newest first. */
    var downloads by mutableStateOf<List<Downloaded>>(emptyList())
        private set

    /** Songs being downloaded now. */
    var downloading by mutableStateOf<Set<String>>(emptySet())
        private set
    var syncing by mutableStateOf(false)
        private set

    /** Last failure of a sync or of a write back to YouTube. */
    var error by mutableStateOf<String?>(null)

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private val disk = Dispatchers.IO.limitedParallelism(1)
    private val file = File(context.filesDir, "library.json")
    val downloadsDir: File = File(context.filesDir, "downloads").apply { mkdirs() }
    private val online get() = App.instance.network.online.value

    private val signedIn get() = catalog.signedIn

    init {
        runCatching { load(JSONObject(file.readText())) }
    }

    // Storage

    private fun load(o: JSONObject) {
        fun tracks(key: String) = o.arr(key)?.objects()?.mapNotNull(Track::fromJson).orEmpty()
        liked = tracks("liked")
        history = tracks("history")
        plays = o.optJSONObject("plays")?.let { p -> p.keys().asSequence().associateWith { p.optInt(it) } }.orEmpty()
        playlists = o.arr("playlists")?.objects()?.map { p ->
            LocalPlaylist(
                id = p.str("id") ?: UUID.randomUUID().toString(),
                name = p.optString("name"),
                tracks = p.arr("tracks")?.objects()?.mapNotNull(Track::fromJson).orEmpty(),
                remoteId = p.str("remoteId"),
                setVideoIds = p.arr("setVideoIds")?.let { a -> (0 until a.length()).map { a.optString(it) } }.orEmpty(),
            )
        }.orEmpty()
        saved = o.arr("saved")?.objects()?.mapNotNull { s ->
            SavedPage(
                id = s.str("id") ?: return@mapNotNull null,
                kind = runCatching { ItemKind.valueOf(s.optString("kind")) }.getOrDefault(ItemKind.Playlist),
                title = s.optString("title"),
                subtitle = s.optString("subtitle"),
                thumbnail = s.str("thumbnail"),
                synced = s.optBoolean("synced"),
            )
        }.orEmpty()
        // Keep only what is still on disk.
        downloads = o.arr("downloads")?.objects()?.mapNotNull { d ->
            val track = d.optJSONObject("track")?.let(Track::fromJson) ?: return@mapNotNull null
            val name = d.str("file") ?: return@mapNotNull null
            if (!File(downloadsDir, name).exists()) return@mapNotNull null
            Downloaded(
                track, name, d.optLong("bytes"),
                (d.opt("duration") as? Number)?.toDouble(), (d.opt("loudnessDb") as? Number)?.toDouble(),
                d.str("exported"),
            )
        }.orEmpty()
    }

    private fun save() {
        val o = JSONObject()
        o.put("liked", JSONArray(liked.map { it.toJson() }))
        o.put("history", JSONArray(history.map { it.toJson() }))
        o.put("plays", JSONObject(plays as Map<*, *>))
        o.put(
            "playlists",
            JSONArray(
                playlists.map { p ->
                    JSONObject().put("id", p.id).put("name", p.name).put("remoteId", p.remoteId)
                        .put("tracks", JSONArray(p.tracks.map { it.toJson() }))
                        .put("setVideoIds", JSONArray(p.setVideoIds))
                },
            ),
        )
        o.put(
            "saved",
            JSONArray(
                saved.map { s ->
                    JSONObject().put("id", s.id).put("kind", s.kind.name).put("title", s.title)
                        .put("subtitle", s.subtitle).put("thumbnail", s.thumbnail).put("synced", s.synced)
                },
            ),
        )
        o.put(
            "downloads",
            JSONArray(
                downloads.map { d ->
                    JSONObject().put("track", d.track.toJson()).put("file", d.file).put("bytes", d.bytes)
                        .put("duration", d.duration).put("loudnessDb", d.loudnessDb).put("exported", d.exported)
                },
            ),
        )
        val text = o.toString()
        scope.launch(disk) {
            val tmp = File(file.parentFile, "library.json.tmp")
            tmp.writeText(text)
            tmp.renameTo(file)
        }
    }

    // The account

    /** True before the first sync of a signed-in account. */
    val needsFirstSync: Boolean get() = signedIn && liked.isEmpty() && playlists.none { it.remoteId != null }

    /**
     * Replaces the account's part of the library with what YouTube Music
     * has. Anything made on the phone while signed out is kept.
     */
    suspend fun sync() {
        if (!signedIn || syncing) return
        syncing = true
        try {
            val likedSongs = catalog.likedSongs()
            val remote = catalog.libraryPlaylists().map { item ->
                val entries = catalog.playlistEntries(item.browseId)
                LocalPlaylist(
                    name = item.title, tracks = entries.map { it.first },
                    remoteId = item.browseId.removePrefix("VL"), setVideoIds = entries.map { it.second },
                )
            }
            val pages = catalog.libraryAlbums() + catalog.libraryArtists()
            liked = likedSongs
            // Keep the local ids of playlists that were already here (open screens point at them).
            val known = playlists.filter { it.remoteId != null }.associateBy { it.remoteId }
            playlists = remote.map { r -> known[r.remoteId]?.let { r.copy(id = it.id) } ?: r } +
                playlists.filter { it.remoteId == null }
            val fromAccount = pages.map { SavedPage(it.browseId, it.kind, it.title, it.subtitle, it.thumbnail, synced = true) }
            val ids = fromAccount.map { it.id }.toSet()
            saved = fromAccount + saved.filter { !it.synced && it.id !in ids }
            save()
            error = null
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            error = context.getString(R.string.sync_failed, e.describe(context, online))
        } finally {
            syncing = false
        }
    }

    /** Forgets what the account contributed (after signing out). */
    fun forgetAccount() {
        liked = emptyList()
        playlists = playlists.filter { it.remoteId == null }
        saved = saved.filter { !it.synced }
        save()
    }

    /**
     * Sends a library change to YouTube in the background; signed out, the
     * change only lives on the phone. `undo` puts the local state back when
     * YouTube refuses it.
     */
    private fun write(what: String, undo: (() -> Unit)? = null, body: suspend () -> Unit) {
        if (!signedIn) return
        scope.launch {
            try {
                body()
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                error = context.getString(R.string.write_failed, what, e.describe(context, online))
                undo?.invoke()
            }
        }
    }

    // Songs

    fun isLiked(track: Track): Boolean = liked.any { it.videoId == track.videoId }

    fun toggleLike(track: Track) {
        val liking = !isLiked(track)
        liked = if (liking) listOf(track) + liked else liked.filter { it.videoId != track.videoId }
        save()
        write(
            context.getString(if (liking) R.string.liking else R.string.unliking, track.title),
            undo = {
                liked = if (liking) liked.filter { it.videoId != track.videoId } else listOf(track) + liked
                save()
            },
        ) { catalog.rate(track.videoId, if (liking) Rating.Like else Rating.None) }
    }

    /** Dislikes a song (YouTube learns from it and drops the like). */
    fun dislike(track: Track) {
        liked = liked.filter { it.videoId != track.videoId }
        save()
        write(context.getString(R.string.disliking, track.title)) { catalog.rate(track.videoId, Rating.Dislike) }
    }

    fun played(track: Track) {
        plays = plays + (track.videoId to (plays[track.videoId] ?: 0) + 1)
        history = (listOf(track) + history.filter { it.videoId != track.videoId }).take(HISTORY_SIZE)
        save()
    }

    fun clearHistory() {
        history = emptyList()
        // Like the desktop player: counts go with the history.
        plays = emptyMap()
        save()
    }

    // Lists

    fun tracks(list: LocalList): List<Track> = when (list) {
        LocalList.Liked -> liked
        LocalList.History -> history
        LocalList.Downloads -> downloads.map { it.track }
        is LocalList.Playlist -> playlists.firstOrNull { it.id == list.id }?.tracks.orEmpty()
    }

    fun title(list: LocalList): String = when (list) {
        LocalList.Liked -> context.getString(if (signedIn) R.string.liked_music else R.string.liked_songs)
        LocalList.History -> context.getString(R.string.history)
        LocalList.Downloads -> context.getString(R.string.downloads)
        is LocalList.Playlist -> playlists.firstOrNull { it.id == list.id }?.name ?: context.getString(R.string.playlist)
    }

    fun remove(at: Int, list: LocalList) {
        when (list) {
            LocalList.Liked -> liked.getOrNull(at)?.let { toggleLike(it) }
            LocalList.History -> {
                history = history.filterIndexed { i, _ -> i != at }
                save()
            }
            LocalList.Downloads -> downloads.getOrNull(at)?.let { removeDownload(it.track) }
            is LocalList.Playlist -> {
                val i = playlists.indexOfFirst { it.id == list.id }.takeIf { it >= 0 } ?: return
                val p = playlists[i]
                val track = p.tracks.getOrNull(at) ?: return
                val setVideoId = p.setVideoIds.getOrNull(at).orEmpty()
                update(p.id) {
                    it.copy(
                        tracks = it.tracks.filterIndexed { j, _ -> j != at },
                        setVideoIds = it.setVideoIds.filterIndexed { j, _ -> j != at },
                    )
                }
                val remoteId = p.remoteId ?: return
                if (setVideoId.isEmpty()) return
                write(context.getString(R.string.removing, track.title), undo = { update(p.id) { p } }) {
                    catalog.removeFromPlaylist(remoteId, track.videoId, setVideoId)
                }
            }
        }
    }

    /** Moves a song within a playlist, here and, for the account's own, on YouTube. */
    fun move(list: LocalList, from: Int, to: Int) {
        val id = (list as? LocalList.Playlist)?.id ?: return
        val before = playlists.firstOrNull { it.id == id } ?: return
        if (from !in before.tracks.indices || to !in before.tracks.indices || from == to) return
        val ids = before.setVideoIds + List(maxOf(0, before.tracks.size - before.setVideoIds.size)) { "" }
        fun <T> List<T>.moved(): List<T> = toMutableList().apply { add(to, removeAt(from)) }
        val moved = before.copy(tracks = before.tracks.moved(), setVideoIds = ids.moved())
        update(id) { moved }
        val remoteId = before.remoteId ?: return
        val setVideoId = moved.setVideoIds[to].ifEmpty { return }
        val successor = moved.setVideoIds.getOrNull(to + 1)?.ifEmpty { null }
        write(context.getString(R.string.moving_song), undo = { update(id) { before } }) {
            catalog.moveInPlaylist(remoteId, setVideoId, successor)
        }
    }

    private fun update(id: String, change: (LocalPlaylist) -> LocalPlaylist) {
        playlists = playlists.map { if (it.id == id) change(it) else it }
        save()
    }

    fun createPlaylist(name: String, with: List<Track> = emptyList()): String {
        val title = name.trim().ifEmpty { context.getString(R.string.playlist_n, playlists.size + 1) }
        val playlist = LocalPlaylist(name = title, tracks = with, setVideoIds = with.map { "" })
        playlists = playlists + playlist
        save()
        write(
            context.getString(R.string.creating, title),
            undo = {
                playlists = playlists.filter { it.id != playlist.id }
                save()
            },
        ) {
            val remoteId = catalog.createPlaylist(title, with.map { it.videoId })
            update(playlist.id) { it.copy(remoteId = remoteId) }
            // The new entries' set ids come with the next sync.
            if (with.isNotEmpty()) sync()
        }
        return playlist.id
    }

    fun deletePlaylist(id: String) {
        val p = playlists.firstOrNull { it.id == id } ?: return
        playlists = playlists.filter { it.id != id }
        save()
        val remoteId = p.remoteId ?: return
        write(context.getString(R.string.deleting, p.name)) { catalog.deletePlaylist(remoteId) }
    }

    fun rename(id: String, name: String) {
        val p = playlists.firstOrNull { it.id == id } ?: return
        val title = name.trim().ifEmpty { return }
        update(id) { it.copy(name = title) }
        val remoteId = p.remoteId ?: return
        write(context.getString(R.string.renaming, p.name), undo = { update(id) { p } }) {
            catalog.renamePlaylist(remoteId, title)
        }
    }

    fun add(track: Track, playlistId: String) {
        val p = playlists.firstOrNull { it.id == playlistId } ?: return
        if (p.tracks.any { it.videoId == track.videoId }) return
        update(playlistId) { it.copy(tracks = it.tracks + track, setVideoIds = it.setVideoIds + "") }
        val remoteId = p.remoteId ?: return
        write(
            context.getString(R.string.adding, track.title),
            undo = { update(playlistId) { cur -> cur.copy(tracks = cur.tracks.dropLast(1), setVideoIds = cur.setVideoIds.dropLast(1)) } },
        ) {
            val setVideoId = catalog.addToPlaylist(remoteId, track.videoId)
            update(playlistId) { cur ->
                val at = cur.tracks.indexOfLast { it.videoId == track.videoId }
                if (at < 0) cur else cur.copy(
                    setVideoIds = (cur.setVideoIds + List(maxOf(0, at + 1 - cur.setVideoIds.size)) { "" })
                        .toMutableList().also { it[at] = setVideoId },
                )
            }
        }
    }

    // Downloads

    fun isDownloaded(videoId: String): Boolean = downloads.any { it.track.videoId == videoId }

    fun download(videoId: String): Pair<File, Downloaded>? {
        val item = downloads.firstOrNull { it.track.videoId == videoId } ?: return null
        return File(downloadsDir, item.file) to item
    }

    /** Songs waiting for their turn to download, in order. */
    private val queue = ArrayDeque<Track>()
    private var worker: Job? = null

    /** How far a batch got: done of total (for "Downloading 3 of 12"). */
    var batchDone by mutableStateOf(0)
        private set
    var batchTotal by mutableStateOf(0)
        private set

    /** Saves the song's audio on the phone (queued behind the others). */
    fun download(track: Track) = download(listOf(track))

    /**
     * Saves songs on the phone one after another (one stream at a time, like
     * the desktop player), in 1 MB ranges — what googlevideo answers reliably.
     */
    fun download(tracks: List<Track>) {
        val fresh = tracks.distinctBy { it.videoId }.filter { !isDownloaded(it.videoId) && it.videoId !in downloading }
        if (fresh.isEmpty()) return
        if (queue.isEmpty() && worker?.isActive != true) {
            batchDone = 0
            batchTotal = 0
        }
        queue.addAll(fresh)
        batchTotal += fresh.size
        downloading = downloading + fresh.map { it.videoId }
        if (worker?.isActive != true) worker = scope.launch { work() }
    }

    private suspend fun work() {
        var failures = 0
        while (true) {
            val track = queue.removeFirstOrNull() ?: break
            try {
                fetchSong(track)
                failures = 0
            } catch (e: CancellationException) {
                File(downloadsDir, "${track.videoId}.m4a.part").delete()
                throw e
            } catch (e: Exception) {
                error = context.getString(R.string.download_failed, e.describe(context, online))
                // No connection: the rest would fail the same way.
                if (!online || ++failures >= 3) {
                    queue.clear()
                    downloading = emptySet()
                    break
                }
            } finally {
                downloading = downloading - track.videoId
                batchDone++
            }
        }
    }

    private suspend fun fetchSong(track: Track) {
        // The studio song's sound, also for a clip (as it plays).
        val stream = resolver.stream(Pairs.audioId(track))
        if (stream.hls) throw YTMError.Unplayable(context.getString(R.string.download_age_restricted))
        val name = "${track.videoId}.m4a"
        val file = File(downloadsDir, name)
        val bytes = withContext(Dispatchers.IO) { fetch(stream.url, stream.contentLength, file) }
        val exported = if (App.instance.settings.exportDownloads) withContext(Dispatchers.IO) { export(track, file) } else null
        downloads = listOf(Downloaded(track, name, bytes, stream.duration, stream.loudnessDb, exported)) +
            downloads.filter { it.track.videoId != track.videoId }
        save()
        Covers.fetch(track)
        keepLyrics(track, stream.duration)
    }

    val downloadedIds: Set<String> get() = downloads.mapTo(HashSet()) { it.track.videoId }

    /** A downloaded song's lyrics, kept with it for offline play. */
    private suspend fun keepLyrics(track: Track, duration: Double?) {
        if (KeptLyrics.contains(track.videoId)) return
        runCatching { KeptLyrics.find(track, duration ?: track.durationSecs?.toDouble(), downloadedIds) }
    }

    /** Songs downloaded before covers and lyrics were kept with them get theirs (once online). */
    suspend fun fetchMissingExtras() {
        downloads.forEach {
            if (Covers.local(it.track.videoId) == null) Covers.fetch(it.track)
            keepLyrics(it.track, it.duration)
        }
    }

    /** Stops what is downloading and forgets what was waiting. */
    fun cancelDownloads() {
        queue.clear()
        worker?.cancel()
        worker = null
        downloading = emptySet()
        batchTotal = 0
        batchDone = 0
    }

    fun removeDownload(track: Track) {
        val (file, item) = download(track.videoId) ?: return
        file.delete()
        unexport(item)
        Covers.remove(track.videoId)
        KeptLyrics.remove(track.videoId)
        downloads = downloads.filter { it.track.videoId != track.videoId }
        save()
    }

    /** Deletes every downloaded song (and its copy in Music). */
    fun removeAllDownloads() {
        cancelDownloads()
        downloads.forEach { item ->
            File(downloadsDir, item.file).delete()
            unexport(item)
            Covers.remove(item.track.videoId)
            KeptLyrics.remove(item.track.videoId)
        }
        downloadsDir.listFiles()?.forEach { it.delete() }
        downloads = emptyList()
        save()
    }

    /**
     * A copy in the phone's Music folder ("Music/Yarek Music/Artist - Title.m4a"),
     * where other players and file managers see it. Android 10+ (MediaStore,
     * no storage permission).
     */
    private fun export(track: Track, file: File): String? {
        if (android.os.Build.VERSION.SDK_INT < android.os.Build.VERSION_CODES.Q) return null
        return runCatching {
            val resolver = context.contentResolver
            val name = "${track.artist} - ${track.title}".replace(Regex("[\\\\/:*?\"<>|]"), "_").take(120) + ".m4a"
            val values = android.content.ContentValues().apply {
                put(android.provider.MediaStore.Audio.Media.DISPLAY_NAME, name)
                put(android.provider.MediaStore.Audio.Media.MIME_TYPE, "audio/mp4")
                put(android.provider.MediaStore.Audio.Media.TITLE, track.title)
                put(android.provider.MediaStore.Audio.Media.ARTIST, track.artist)
                put(android.provider.MediaStore.Audio.Media.RELATIVE_PATH, "Music/Yarek Music")
                put(android.provider.MediaStore.Audio.Media.IS_PENDING, 1)
            }
            val uri = resolver.insert(android.provider.MediaStore.Audio.Media.EXTERNAL_CONTENT_URI, values) ?: return null
            resolver.openOutputStream(uri)?.use { out -> file.inputStream().use { it.copyTo(out) } }
            values.clear()
            values.put(android.provider.MediaStore.Audio.Media.IS_PENDING, 0)
            resolver.update(uri, values, null, null)
            uri.toString()
        }.getOrNull()
    }

    private fun unexport(item: Downloaded) {
        val uri = item.exported ?: return
        runCatching { context.contentResolver.delete(android.net.Uri.parse(uri), null, null) }
    }

    /** Fetches `url` to `target` range by range; returns its size. A dropped connection resumes where it stopped. */
    private suspend fun fetch(url: String, length: Long?, target: File): Long {
        val partial = File(target.parentFile, target.name + ".part")
        RandomAccessFile(partial, "rw").use { out ->
            out.setLength(0)
            var offset = 0L
            var failures = 0
            while (true) {
                val end = length?.let { minOf(offset + CHUNK, it) - 1 } ?: (offset + CHUNK - 1)
                val r = try {
                    Http.get(url, mapOf("Range" to "bytes=$offset-$end"))
                } catch (e: java.io.IOException) {
                    // A weak signal: try the same range again a few times.
                    if (++failures > 4) throw e
                    kotlinx.coroutines.delay(1000L * failures)
                    continue
                }
                if (r.status != 206 && r.status != 200) throw YTMError.Http("Download", r.status)
                failures = 0
                out.write(r.body)
                offset += r.body.size
                if (r.body.isEmpty()) break
                if (length != null) {
                    if (offset >= length) break
                } else if (r.body.size < CHUNK) {
                    break
                }
            }
        }
        target.delete()
        partial.renameTo(target)
        return target.length()
    }

    // Saved pages

    fun isSaved(id: String): Boolean = saved.any { it.id == id }

    /** Saves an album or playlist page, or follows an artist (subscribing on YouTube when signed in). */
    fun toggleSaved(page: Page) {
        val saving = !isSaved(page.id)
        saved = if (saving) {
            listOf(SavedPage(page.id, page.kind, page.title, page.subtitle, page.thumbnail, synced = page.kind == ItemKind.Artist && signedIn)) + saved
        } else {
            saved.filter { it.id != page.id }
        }
        save()
        if (page.kind != ItemKind.Artist) return
        write(context.getString(if (saving) R.string.following else R.string.unfollowing, page.title)) {
            catalog.subscribe(page.id, saving)
        }
    }

    fun deleteSaved(id: String) {
        val page = saved.firstOrNull { it.id == id } ?: return
        saved = saved.filter { it.id != id }
        save()
        if (page.kind == ItemKind.Artist) {
            write(context.getString(R.string.unfollowing, page.title)) { catalog.subscribe(page.id, false) }
        }
    }

    companion object {
        const val HISTORY_SIZE = 200
        private const val CHUNK = 1 shl 20
    }
}

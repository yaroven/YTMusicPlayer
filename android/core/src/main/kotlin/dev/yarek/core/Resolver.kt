package dev.yarek.core

import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.delay
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import org.json.JSONArray
import org.json.JSONObject
import java.net.URI

/**
 * An InnerTube client the player endpoint is asked as. Clients that hand
 * out plain (unciphered, no PO token) audio URLs change over time, so the
 * list can be replaced without a new build: `ios/clients.json` in the repo,
 * shared with the iPhone app.
 */
data class ClientProfile(
    val name: String,
    val version: String,
    /** `X-YouTube-Client-Name`. */
    val id: String,
    val userAgent: String,
    val deviceMake: String? = null,
    val deviceModel: String? = null,
    val osName: String? = null,
    val osVersion: String? = null,
    /** Needs `visitorData` from youtube.com. */
    val visitor: Boolean = false,
    /** Sent with the account's cookies, only for songs anonymous clients can't play. */
    val auth: Boolean = false,
    /** Without a plain audio URL, take the HLS manifest's audio playlist. */
    val hls: Boolean = false,
    /** Web clients: formats are ciphered; solve YouTube's JS challenges. */
    val cipher: Boolean = false,
    /** Formats to take, in order (default: 140, the AAC audio). */
    val itags: List<Int>? = null,
) {
    val label: String get() = name + if (auth) " (signed in)" else ""

    internal fun context(): JSONObject = json(
        "clientName" to name, "clientVersion" to version, "userAgent" to userAgent, "hl" to "en", "gl" to "US",
        "deviceMake" to deviceMake, "deviceModel" to deviceModel, "osName" to osName, "osVersion" to osVersion,
    )

    companion object {
        fun fromJson(o: JSONObject): ClientProfile? {
            val name = o.str("name") ?: return null
            val version = o.str("version") ?: return null
            val id = o.str("id") ?: return null
            val userAgent = o.str("userAgent") ?: return null
            return ClientProfile(
                name = name,
                version = version,
                id = id,
                userAgent = userAgent,
                deviceMake = o.str("deviceMake"),
                deviceModel = o.str("deviceModel"),
                osName = o.str("osName"),
                osVersion = o.str("osVersion"),
                visitor = o.optBoolean("visitor"),
                auth = o.optBoolean("auth"),
                hls = o.optBoolean("hls"),
                cipher = o.optBoolean("cipher"),
                itags = o.arr("itags")?.let { a -> (0 until a.length()).map { a.getInt(it) } },
            )
        }

        fun listFromJson(text: String): List<ClientProfile> =
            JSONArray(text).objects().mapNotNull(::fromJson)

        /** Measured 2026-10-12: itag 140 URL, every range 206. */
        val visionOS = ClientProfile(
            name = "VISIONOS", version = "1.02", id = "101",
            userAgent = "Mozilla/5.0 (Macintosh; Intel Mac OS X 15_7_3) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.0 Safari/605.1.15",
            deviceMake = "Apple", deviceModel = "RealityDevice17,1",
            osName = "visionOS", osVersion = "26.5.23O471", visitor = true,
        )

        /** Signed-in web clients for age-restricted songs (see CLAUDE.md). */
        val signedIn = listOf(
            ClientProfile(
                name = "WEB", version = "2.20260708.00.00", id = "1",
                userAgent = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/15.5 Safari/605.1.15,gzip(gfe)",
                visitor = true, auth = true, hls = true, cipher = true, itags = emptyList(),
            ),
            ClientProfile(
                name = "MWEB", version = "2.20260708.05.00", id = "2",
                userAgent = "Mozilla/5.0 (iPad; CPU OS 16_7_10 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/16.6 Mobile/15E148 Safari/604.1",
                visitor = true, auth = true, cipher = true, itags = listOf(18),
            ),
        )

        val builtIn = listOf(visionOS) + signedIn
    }
}

/** A playable audio URL; googlevideo URLs are bound to this IP and expire after ~6 h. */
data class Stream(
    val url: String,
    val expiresAtMillis: Long,
    val contentLength: Long? = null,
    /** The real length in seconds. */
    val duration: Double? = null,
    /** Loudness relative to YouTube's target in dB (positive: louder). */
    val loudnessDb: Double? = null,
    /** `url` is an HLS playlist. */
    val hls: Boolean = false,
    /** A muxed video (itag 18 / HLS variant): only its audio should play. */
    val hasVideo: Boolean = false,
    /** What fetched it; googlevideo accepts it for the media requests. */
    val userAgent: String = "",
)

/**
 * Video id → stream URL on the device itself (no yt-dlp): asks the player
 * endpoint as each `ClientProfile` until one returns the AAC stream.
 * Port of YTMKit's Resolver.swift.
 */
class Resolver(private val auth: Auth? = null, js: JsEngine? = null) {
    private val mutex = Mutex()
    private var profiles = ClientProfile.builtIn
    private var fetchedProfiles = false
    private val visitors = HashMap<Boolean, Pair<String, Long>>()
    private val cache = LinkedHashMap<String, Stream>()
    private var playerUrl: String? = null
    private val solver = ChallengeSolver(js)

    suspend fun stream(videoId: String): Stream {
        mutex.withLock {
            cache[videoId]?.let { if (it.expiresAtMillis - System.currentTimeMillis() > 600_000) return it }
        }
        loadRemoteProfiles()
        var lastError: Throwable = YTMError.NoStream
        var needsSignIn = false
        for (profile in profiles.filter { !it.auth }) {
            try {
                return remember(videoId, resolve(videoId, profile))
            } catch (e: CancellationException) {
                throw e
            } catch (e: YTMError.SignInRequired) {
                needsSignIn = true
                lastError = e
            } catch (e: Exception) {
                lastError = e
            }
        }
        if (!needsSignIn) throw lastError
        if (auth?.signedIn != true) {
            throw YTMError.SignInRequired("Age-restricted: sign in with Google to play it")
        }
        for (profile in profiles.filter { it.auth }) {
            try {
                return remember(videoId, resolve(videoId, profile))
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                lastError = e
            }
        }
        throw lastError
    }

    /** One line per client: what it answers and whether the audio downloads. */
    suspend fun diagnose(videoId: String): List<String> {
        loadRemoteProfiles()
        val lines = ArrayList<String>()
        for (profile in profiles.filter { !it.auth || auth?.signedIn == true }) {
            try {
                val stream = resolve(videoId, profile)
                val middle = (stream.contentLength ?: 0) / 2
                val headers = if (stream.hls) emptyMap() else mapOf("Range" to "bytes=$middle-${middle + 65_535}")
                val status = runCatching { Http.get(stream.url, headers).status }.getOrNull()
                val ok = status == 206 || (stream.hls && status == 200)
                lines += "${if (ok) "✓" else "✗"} ${profile.label}: ${if (stream.hls) "HLS" else "AAC"} → HTTP ${status ?: "error"}"
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                lines += "✗ ${profile.label}: ${e.message}"
            }
        }
        if (auth?.signedIn != true) lines += "Not signed in: signed-in clients skipped"
        return lines
    }

    /** Drop a URL that stopped working (403/410, network change). */
    suspend fun forget(videoId: String) = mutex.withLock {
        cache.remove(videoId)
        visitors.clear()
    }

    /** After signing in or out. */
    suspend fun reset() = mutex.withLock {
        cache.clear()
        visitors.clear()
    }

    private suspend fun remember(videoId: String, stream: Stream): Stream = mutex.withLock {
        cache.remove(videoId)
        cache[videoId] = stream
        while (cache.size > CACHE_SIZE) cache.remove(cache.keys.first())
        stream
    }

    private suspend fun loadRemoteProfiles() {
        if (fetchedProfiles) return
        fetchedProfiles = true
        val remote = runCatching {
            val r = Http.send(REMOTE_PROFILES, headers = mapOf("Cache-Control" to "no-cache"), readTimeoutMs = 5_000)
            if (r.status == 200) ClientProfile.listFromJson(r.text) else null
        }.getOrNull()
        if (!remote.isNullOrEmpty()) profiles = remote
    }

    private suspend fun visitorData(videoId: String, profile: ClientProfile, signedIn: Boolean): String? {
        mutex.withLock {
            visitors[signedIn]?.let { (value, at) -> if (System.currentTimeMillis() - at < 3_600_000) return value }
        }
        val headers = mapOf("User-Agent" to profile.userAgent, "Accept-Language" to "en-US,en")
        val page = send("https://www.youtube.com/watch?v=$videoId", "GET", headers, null, signedIn).text
        ChallengeSolver.playerUrl(page)?.let { playerUrl = it }
        val value = visitorDataIn(page) ?: return null
        mutex.withLock { visitors[signedIn] = value to System.currentTimeMillis() }
        return value
    }

    /** Signed-in requests carry the account's cookies; anonymous ones never do. */
    private suspend fun send(url: String, method: String, headers: Map<String, String>, body: ByteArray?, signedIn: Boolean): Response {
        val signer = auth?.takeIf { signedIn }
        val all = signer?.let { headers + it.headers("https://www.youtube.com") } ?: headers
        val response = Http.send(url, method, all, body)
        signer?.absorb(response)
        return response
    }

    private suspend fun resolve(videoId: String, profile: ClientProfile): Stream {
        val signedIn = profile.auth
        val client = profile.context()
        val headers = mutableMapOf(
            "Content-Type" to "application/json",
            "User-Agent" to profile.userAgent,
            "X-YouTube-Client-Name" to profile.id,
            "X-YouTube-Client-Version" to profile.version,
            "Origin" to "https://www.youtube.com",
        )
        if (profile.visitor) {
            visitorData(videoId, profile, signedIn)?.let {
                client.put("visitorData", it)
                headers["X-Goog-Visitor-Id"] = it
            }
        }
        val body = json("videoId" to videoId, "contentCheckOk" to true, "racyCheckOk" to true)
        body.put("context", JSONObject().put("client", client))
        if (profile.cipher) {
            if (playerUrl == null) visitorData(videoId, profile, signedIn)
            val player = playerUrl ?: throw YTMError.Unplayable("YouTube's player script not found")
            solver.signatureTimestamp(player)?.let {
                body.put("playbackContext", json("contentPlaybackContext" to mapOf("signatureTimestamp" to it)))
            }
        }
        val response = send(
            "https://www.youtube.com/youtubei/v1/player?prettyPrint=false", "POST", headers,
            body.toString().toByteArray(), signedIn,
        )
        if (response.status != 200) throw YTMError.Http("YouTube player", response.status)
        var data = JSONObject(response.text)
        val player = playerUrl
        if (profile.cipher && player != null && string(data, "playabilityStatus/status") == "OK") {
            data = decipherFormats(data, profile.itags ?: listOf(140), player)
        }
        var stream = parsePlayer(data, profile.hls, profile.itags ?: listOf(140)).copy(userAgent = profile.userAgent)
        if (stream.hls) {
            if (profile.cipher && player != null) stream = stream.copy(url = solveN(stream.url, player))
            // The master lists several variants; take the audio-only
            // rendition, else the smallest variant with AAC-LC audio.
            val master = Http.get(stream.url, mapOf("User-Agent" to profile.userAgent)).text
            val audio = audioPlaylist(master, stream.url)
            stream = when {
                audio != null -> stream.copy(url = audio)
                else -> aacVariant(master, stream.url)?.let { stream.copy(url = it, hasVideo = true) } ?: stream
            }
            waitForSegments(stream.url)
        }
        return stream
    }

    /** googlevideo HLS URLs carry "n" as a path part (`/n/<value>/`). */
    private suspend fun solveN(url: String, player: String): String {
        val match = Regex("/n/([^/]+)/").find(url) ?: return url
        val value = match.groupValues[1]
        val solved = solver.solve(player, emptyList(), listOf(value)).second[value] ?: return url
        return url.replaceRange(match.range, "/n/$solved/")
    }

    /** A fresh playlist's segments answer 403 for the first seconds; wait until the first is ready. */
    private suspend fun waitForSegments(playlist: String) {
        val text = Http.get(playlist).text
        val first = text.lines().firstOrNull { it.isNotEmpty() && !it.startsWith("#") } ?: return
        val url = resolveUrl(playlist, first)
        var wait = 500L
        repeat(5) {
            val status = runCatching { Http.get(url, mapOf("Range" to "bytes=0-1023")).status }.getOrDefault(0)
            if (status == 200 || status == 206) return
            delay(wait)
            wait *= 2
        }
    }

    /** Plain URLs for the wanted formats of a web client's response. */
    private suspend fun decipherFormats(data: JSONObject, itags: List<Int>, player: String): JSONObject {
        val streaming = data.obj("streamingData") ?: return data
        class Pending(val format: JSONObject, val url: String, val s: String?, val sp: String)
        val pending = ArrayList<Pending>()
        for (key in listOf("adaptiveFormats", "formats")) {
            for (format in streaming.arr(key)?.objects().orEmpty()) {
                if ((format.opt("itag") as? Number)?.toInt() !in itags) continue
                val plain = format.str("url")
                if (plain != null) {
                    pending += Pending(format, plain, null, "sig")
                } else {
                    val cipher = format.str("signatureCipher") ?: continue
                    val query = parseQuery(cipher)
                    val raw = query["url"] ?: continue
                    pending += Pending(format, raw, query["s"], query["sp"] ?: "signature")
                }
            }
        }
        val ns = pending.mapNotNull { parseQuery(it.url.substringAfter('?', ""))["n"] }.distinct()
        val (sigs, nsSolved) = solver.solve(player, pending.mapNotNull { it.s }, ns)
        for (item in pending) {
            var url = item.url
            if (item.s != null) {
                val signature = sigs[item.s] ?: continue
                url += "&${item.sp}=" + java.net.URLEncoder.encode(signature, "UTF-8")
            }
            val n = parseQuery(url.substringAfter('?', ""))["n"]
            n?.let { nsSolved[it] }?.let { url = withParam(url, "n", it) }
            item.format.put("url", url)
            item.format.remove("signatureCipher")
        }
        return data
    }

    companion object {
        const val REMOTE_PROFILES = "https://raw.githubusercontent.com/yaroven/YTMusicPlayer/main/ios/clients.json"
        const val CACHE_SIZE = 16
    }
}

internal fun parseQuery(query: String): Map<String, String> = query.split('&').mapNotNull {
    val eq = it.indexOf('=')
    if (eq <= 0) null else it.substring(0, eq) to java.net.URLDecoder.decode(it.substring(eq + 1), "UTF-8")
}.toMap()

/** `url` with the query parameter `name` set to `value`. */
internal fun withParam(url: String, name: String, value: String): String {
    val base = url.substringBefore('?')
    val parts = url.substringAfter('?', "").split('&').filter { it.isNotEmpty() && it.substringBefore('=') != name }
    return base + "?" + (parts + ("$name=" + java.net.URLEncoder.encode(value, "UTF-8"))).joinToString("&")
}

internal fun resolveUrl(base: String, ref: String): String = URI(base).resolve(ref.trim()).toString()

/** Loudness from the track's measurement when the format lacks one. */
fun trackLoudness(data: Any): Double? {
    val track = (pointer(data, "playerConfig/audioConfig/trackAbsoluteLoudnessLkfs") as? Number)?.toDouble()
    val target = (pointer(data, "playerConfig/audioConfig/loudnessTargetLkfs") as? Number)?.toDouble()
    if (track != null && target != null) return track - target
    return (pointer(data, "playerConfig/audioConfig/loudnessDb") as? Number)?.toDouble()
}

fun visitorDataIn(html: String): String? {
    val start = html.indexOf("\"VISITOR_DATA\":\"").takeIf { it >= 0 } ?: return null
    val from = start + "\"VISITOR_DATA\":\"".length
    val end = html.indexOf('"', from).takeIf { it >= 0 } ?: return null
    return html.substring(from, end).ifEmpty { null }
}

/** An HLS master playlist's audio-only rendition (AAC itag 140 if listed). */
fun audioPlaylist(master: String, base: String): String? {
    val uris = master.lines()
        .filter { it.startsWith("#EXT-X-MEDIA:") && it.contains("TYPE=AUDIO") }
        .mapNotNull { Regex("URI=\"([^\"]+)\"").find(it)?.groupValues?.get(1) }
    val pick = uris.firstOrNull { it.contains("/itag/140/") } ?: uris.firstOrNull()
    return pick?.let { resolveUrl(base, it) }
}

/** The smallest HLS variant with AAC-LC audio (`mp4a.40.2`). */
fun aacVariant(master: String, base: String): String? {
    val lines = master.lines()
    var best: Pair<Long, String>? = null
    for ((i, line) in lines.withIndex()) {
        if (!line.startsWith("#EXT-X-STREAM-INF") || !line.contains("mp4a.40.2") || i + 1 >= lines.size) continue
        val bandwidth = Regex("BANDWIDTH=(\\d+)").find(line)?.groupValues?.get(1)?.toLongOrNull() ?: Long.MAX_VALUE
        if (best == null || bandwidth < best.first) best = bandwidth to resolveUrl(base, lines[i + 1])
    }
    return best?.second
}

/** The AAC (itag 140) stream of a player response; with `hls`, the HLS manifest when no plain URL is there. */
fun parsePlayer(data: Any, hls: Boolean = false, itags: List<Int> = listOf(140), nowMillis: Long = System.currentTimeMillis()): Stream {
    val status = string(data, "playabilityStatus/status") ?: ""
    if (status != "OK") {
        val reason = string(data, "playabilityStatus/reason") ?: status
        val message = reason.ifEmpty { "This song can't be played" }
        throw if (status == "LOGIN_REQUIRED") YTMError.SignInRequired(message) else YTMError.Unplayable(message)
    }
    val length = string(data, "videoDetails/lengthSeconds")?.toDoubleOrNull()
    val expiresIn = string(data, "streamingData/expiresInSeconds")?.toDoubleOrNull()
    val formats = ((pointer(data, "streamingData/adaptiveFormats") as? JSONArray)?.objects().orEmpty()) +
        ((pointer(data, "streamingData/formats") as? JSONArray)?.objects().orEmpty())
    fun itag(f: JSONObject) = (f.opt("itag") as? Number)?.toInt()
    val wanted = itags.firstNotNullOfOrNull { want -> formats.firstOrNull { itag(it) == want && it.str("url") != null } }
    val aac = wanted ?: formats.firstOrNull { (it.str("mimeType") ?: "").startsWith("audio/mp4") && it.str("url") != null }
    val url = aac?.str("url")
    if (aac == null || url == null) {
        val manifest = string(data, "streamingData/hlsManifestUrl")
        if (hls && manifest != null) {
            return Stream(manifest, nowMillis + ((expiresIn ?: 3600.0) * 1000).toLong(), duration = length, loudnessDb = trackLoudness(data), hls = true)
        }
        throw YTMError.NoStream
    }
    val query = parseQuery(url.substringAfter('?', ""))
    val expires = expiresIn ?: query["expire"]?.toDoubleOrNull()?.let { it - nowMillis / 1000.0 } ?: 3600.0
    val loudness = (aac.opt("loudnessDb") as? Number)?.toDouble() ?: trackLoudness(data)
    val duration = aac.str("approxDurationMs")?.toDoubleOrNull()?.let { it / 1000 }
        ?: query["dur"]?.toDoubleOrNull() ?: length
    return Stream(
        url = url,
        expiresAtMillis = nowMillis + (expires * 1000).toLong(),
        contentLength = aac.str("contentLength")?.toLongOrNull(),
        duration = duration,
        loudnessDb = loudness,
        hasVideo = (aac.str("mimeType") ?: "").startsWith("video/"),
    )
}

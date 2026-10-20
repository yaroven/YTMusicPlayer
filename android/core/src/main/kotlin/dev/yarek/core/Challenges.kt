package dev.yarek.core

import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import org.json.JSONObject

/**
 * Runs yt-dlp's challenge solver. On Android a headless WebView
 * (`app`'s `WebViewJs`); there is none in the JVM checks.
 */
interface JsEngine {
    /**
     * Evaluates `script` (which defines `jsc`) in a fresh context, then
     * returns `JSON.stringify(jsc(JSON.parse(input)))`.
     */
    suspend fun solve(script: String, input: String): String
}

/**
 * YouTube's signature ("s") and throttling ("n") challenges, solved with
 * yt-dlp's own solver (yt-dlp-ejs 0.8.0, Unlicense; the same files the
 * iPhone app bundles). Needed only for the signed-in web clients that
 * serve age-restricted songs. Port of YTMKit's Challenges.swift.
 */
class ChallengeSolver(private val engine: JsEngine?) {
    private val mutex = Mutex()
    private val preprocessed = HashMap<String, String>()
    private val timestamps = HashMap<String, Int?>()

    suspend fun signatureTimestamp(url: String): Int? {
        mutex.withLock { if (timestamps.containsKey(url)) return timestamps[url] }
        val code = download(url)
        val ts = signatureTimestampIn(code)
        mutex.withLock { timestamps[url] = ts }
        return ts
    }

    /** Solutions for each challenge, by challenge: (sig, n). */
    suspend fun solve(url: String, sig: List<String>, n: List<String>): Pair<Map<String, String>, Map<String, String>> {
        if (sig.isEmpty() && n.isEmpty()) return emptyMap<String, String>() to emptyMap()
        val engine = engine ?: throw YTMError.Unplayable("JavaScript solver unavailable")
        val requests = ArrayList<Map<String, Any>>()
        if (sig.isNotEmpty()) requests += mapOf("type" to "sig", "challenges" to sig)
        if (n.isNotEmpty()) requests += mapOf("type" to "n", "challenges" to n)
        val input = json("requests" to requests)
        val cached = mutex.withLock { preprocessed[url] }
        if (cached != null) {
            input.put("type", "preprocessed").put("preprocessed_player", cached)
        } else {
            input.put("type", "player").put("player", download(url)).put("output_preprocessed", true)
        }
        val output = JSONObject(engine.solve(script, input.toString()))
        if (output.str("type") == "error") throw YTMError.Unplayable("JavaScript solver: ${output.opt("error")}")
        output.str("preprocessed_player")?.let { p -> mutex.withLock { preprocessed[url] = p } }
        val responses = output.arr("responses")?.objects().orEmpty()
        val sigs = HashMap<String, String>()
        val ns = HashMap<String, String>()
        for ((request, response) in requests.zip(responses)) {
            val data = response.obj("data")
            if (response.str("type") != "result" || data == null) {
                throw YTMError.Unplayable("YouTube's player changed (${response.opt("error") ?: "no result"})")
            }
            val target = if (request["type"] == "sig") sigs else ns
            for (k in data.keys()) target[k] = data.getString(k)
        }
        return sigs to ns
    }

    private suspend fun download(url: String): String {
        val r = Http.get(url)
        if (r.status != 200) throw YTMError.Http("YouTube player script", r.status)
        return r.text
    }

    companion object {
        /** yt-dlp-ejs' lib (meriyah, astring) and core, as one script. */
        val script: String by lazy {
            fun load(name: String) = ChallengeSolver::class.java.getResourceAsStream("/$name")
                ?.use { it.readBytes().toString(Charsets.UTF_8) } ?: error("missing $name")
            load("yt.solver.lib.min.js") + "\nObject.assign(globalThis, lib);\n" + load("yt.solver.core.min.js")
        }

        /**
         * `"jsUrl":"/s/player/<id>/player_es6_tcc.vflset/en_US/base.js"` in a
         * watch page, as the "main" variant yt-dlp's solver understands.
         */
        fun playerUrl(html: String): String? {
            val start = html.indexOf("\"jsUrl\":\"").takeIf { it >= 0 } ?: return null
            val from = start + "\"jsUrl\":\"".length
            val end = html.indexOf('"', from).takeIf { it >= 0 } ?: return null
            var path = html.substring(from, end)
            Regex("/s/player/[0-9a-zA-Z_-]+/").find(path)?.let {
                path = path.substring(0, it.range.last + 1) + "player_ias.vflset/en_US/base.js"
            }
            return resolveUrl("https://www.youtube.com", path)
        }

        fun signatureTimestampIn(code: String): Int? {
            for (pattern in listOf("signatureTimestamp[:=](\\d+)", "sts:(\\d+)")) {
                Regex(pattern).find(code)?.groupValues?.get(1)?.toIntOrNull()?.let { return it }
            }
            return null
        }
    }
}

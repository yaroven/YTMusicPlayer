package dev.yarek.core

import java.security.MessageDigest

/**
 * A signed-in YouTube session: the youtube.com cookies the sign-in page
 * left behind. Requests carry them plus the `SAPISIDHASH` authorization the
 * web clients send. Port of YTMKit's Auth.swift.
 */
class Auth(
    cookies: Map<String, String> = emptyMap(),
    /** Called with the updated cookies when YouTube rotates them. */
    private val onChange: (Map<String, String>) -> Unit = {},
) {
    private val lock = Any()
    private var cookies: Map<String, String> = cookies

    val signedIn: Boolean get() = synchronized(lock) { isSignedIn(cookies) }

    fun set(cookies: Map<String, String>) {
        synchronized(lock) { this.cookies = cookies }
    }

    /** Cookie, Authorization and the headers that go with them. */
    fun headers(origin: String, nowSeconds: Long = System.currentTimeMillis() / 1000): Map<String, String> {
        val cookies = synchronized(lock) { cookies }
        if (!isSignedIn(cookies)) return emptyMap()
        return mapOf(
            "Cookie" to cookies.map { "${it.key}=${it.value}" }.sorted().joinToString("; "),
            "Authorization" to authorization(cookies, origin, nowSeconds),
            "X-Origin" to origin,
            "X-Goog-AuthUser" to "0",
            "X-Youtube-Bootstrap-Logged-In" to "true",
        )
    }

    /** Keep cookies YouTube rotates (`__Secure-*PSIDTS` etc.). */
    fun absorb(response: Response) {
        val fresh = response.header("Set-Cookie").mapNotNull { line ->
            val pair = line.substringBefore(';')
            val eq = pair.indexOf('=')
            if (eq <= 0) null else pair.substring(0, eq).trim() to pair.substring(eq + 1).trim()
        }
        if (fresh.isEmpty()) return
        val updated = synchronized(lock) {
            if (!isSignedIn(cookies)) return
            var changed = false
            val next = cookies.toMutableMap()
            for ((name, value) in fresh) {
                if (next.containsKey(name) && next[name] != value) {
                    next[name] = value
                    changed = true
                }
            }
            if (!changed) return
            cookies = next
            next
        }
        onChange(updated)
    }

    companion object {
        /** The cookies that make a youtube.com session. */
        fun isSignedIn(cookies: Map<String, String>): Boolean =
            (cookies["SAPISID"] ?: cookies["__Secure-3PAPISID"]) != null &&
                (cookies["SID"] != null || cookies["__Secure-3PSID"] != null)
    }
}

/**
 * `SAPISIDHASH <ts>_<sha1("<ts> <SAPISID> <origin>")>`, plus the 1P / 3P
 * variants, as youtube.com sends it.
 */
fun authorization(cookies: Map<String, String>, origin: String, nowSeconds: Long): String {
    val ts = nowSeconds.toString()
    fun hash(value: String): String = MessageDigest.getInstance("SHA-1")
        .digest("$ts $value $origin".toByteArray())
        .joinToString("") { "%02x".format(it) }
    val schemes = listOf(
        "SAPISIDHASH" to (cookies["SAPISID"] ?: cookies["__Secure-3PAPISID"]),
        "SAPISID1PHASH" to cookies["__Secure-1PAPISID"],
        "SAPISID3PHASH" to cookies["__Secure-3PAPISID"],
    )
    return schemes.mapNotNull { (scheme, value) -> value?.let { "$scheme ${ts}_${hash(it)}" } }.joinToString(" ")
}

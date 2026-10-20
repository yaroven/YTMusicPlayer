package dev.yarek.core

import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.runInterruptible
import java.io.IOException
import java.io.InterruptedIOException
import java.net.ConnectException
import java.net.HttpURLConnection
import java.net.NoRouteToHostException
import java.net.SocketException
import java.net.URL
import java.net.UnknownHostException
import javax.net.ssl.SSLException

class Response(val status: Int, val headers: Map<String, List<String>>, val body: ByteArray) {
    val text: String get() = body.toString(Charsets.UTF_8)
    val ok: Boolean get() = status in 200..299

    /** Header values by case-insensitive name. */
    fun header(name: String): List<String> =
        headers.entries.filter { it.key.equals(name, ignoreCase = true) }.flatMap { it.value }
}

/**
 * Plain `HttpURLConnection` (no OkHttp): the platform's own stack on
 * Android, and the JVM's for the checks. No cookie store is involved —
 * signed-in requests carry their cookies in the headers built by `Auth`.
 */
object Http {
    /** A phone on a bad signal: give up on a dead connection, not a slow one. */
    const val CONNECT_TIMEOUT_MS = 10_000
    const val READ_TIMEOUT_MS = 20_000

    suspend fun send(
        url: String,
        method: String = "GET",
        headers: Map<String, String> = emptyMap(),
        body: ByteArray? = null,
        readTimeoutMs: Int = READ_TIMEOUT_MS,
    ): Response = runInterruptible(Dispatchers.IO) {
        val connection = URL(url).openConnection() as HttpURLConnection
        try {
            connection.connectTimeout = CONNECT_TIMEOUT_MS
            connection.readTimeout = readTimeoutMs
            connection.requestMethod = method
            connection.instanceFollowRedirects = true
            for ((name, value) in headers) connection.setRequestProperty(name, value)
            if (body != null) {
                connection.doOutput = true
                connection.setFixedLengthStreamingMode(body.size)
                connection.outputStream.use { it.write(body) }
            }
            val status = connection.responseCode
            val stream = if (status >= 400) connection.errorStream else connection.inputStream
            val bytes = stream?.use { it.readBytes() } ?: ByteArray(0)
            Response(status, connection.headerFields.filterKeys { it != null }, bytes)
        } finally {
            connection.disconnect()
        }
    }

    suspend fun get(url: String, headers: Map<String, String> = emptyMap()): Response = send(url, headers = headers)
}

/**
 * The request never reached YouTube or its answer never came back: no
 * connection, DNS failing, a timeout. Not an HTTP error status.
 */
fun Throwable.isNetworkFailure(): Boolean {
    var e: Throwable? = this
    while (e != null) {
        when (e) {
            is UnknownHostException, is ConnectException, is NoRouteToHostException,
            is InterruptedIOException, is SSLException, is SocketException -> return true
            is YTMError -> return false
        }
        e = e.cause
    }
    return this is IOException
}

/** A timeout in particular (the connection is there but slow). */
fun Throwable.isTimeout(): Boolean = this is java.net.SocketTimeoutException ||
    (this is InterruptedIOException && message?.contains("timeout", ignoreCase = true) == true)

package dev.yarek.music

import android.net.Uri
import androidx.annotation.OptIn
import androidx.media3.common.C
import androidx.media3.common.util.UnstableApi
import androidx.media3.datasource.DataSource
import androidx.media3.datasource.DataSpec
import androidx.media3.datasource.HttpDataSource
import androidx.media3.datasource.TransferListener

/**
 * googlevideo throttles big ranges to the song's own bitrate: measured
 * 2026-10-20, an open-ended request for a 23 MB song came at ~32 KB/s (not
 * even its first MB in 15 s; a 39 MB one took 20 minutes), while 1 MB
 * ranges of the same URL came at ~10 MB/s. Short songs (under ~10 MB) were
 * never throttled, which is why only long ones stalled. So ExoPlayer's
 * single open-ended read is served here as a run of `CHUNK`-sized range
 * requests, one after another (yt-dlp does the same with 10 MB chunks; the
 * desktop player reads 256 KB ones).
 */
@OptIn(UnstableApi::class)
class ChunkedHttpDataSource(private val upstream: HttpDataSource) : DataSource {
    private var spec: DataSpec? = null
    /** Where the next chunk starts, and where the whole read ends (exclusive; unknown = -1). */
    private var position = 0L
    private var end = -1L
    private var chunkLeft = 0L
    private var open = false

    override fun addTransferListener(transferListener: TransferListener) = upstream.addTransferListener(transferListener)

    override fun open(dataSpec: DataSpec): Long {
        spec = dataSpec
        position = dataSpec.position
        end = if (dataSpec.length != C.LENGTH_UNSET.toLong()) dataSpec.position + dataSpec.length else -1L
        openChunk()
        if (end < 0) {
            // The whole file's length, from "Content-Range: bytes a-b/total".
            upstream.responseHeaders.entries
                .firstOrNull { it.key.equals("Content-Range", ignoreCase = true) }
                ?.value?.firstOrNull()?.substringAfterLast('/')?.toLongOrNull()
                ?.let { end = it }
        }
        return if (end >= 0) end - dataSpec.position else C.LENGTH_UNSET.toLong()
    }

    /** Opens the range [position, position + CHUNK) (clipped to `end`); returns its length. */
    private fun openChunk(): Long {
        val base = spec ?: error("not open")
        val length = if (end >= 0) minOf(CHUNK, end - position) else CHUNK
        val chunk = base.buildUpon().setPosition(position).setLength(length).build()
        val got = upstream.open(chunk)
        open = true
        chunkLeft = if (got == C.LENGTH_UNSET.toLong()) length else got
        return chunkLeft
    }

    override fun read(buffer: ByteArray, offset: Int, length: Int): Int {
        if (length == 0) return 0
        if (end >= 0 && position >= end) return C.RESULT_END_OF_INPUT
        if (chunkLeft == 0L) {
            upstream.close()
            open = false
            openChunk()
        }
        val n = upstream.read(buffer, offset, minOf(length.toLong(), chunkLeft).toInt())
        if (n == C.RESULT_END_OF_INPUT) {
            // A short chunk: the file ended (its length wasn't known).
            if (end < 0) return C.RESULT_END_OF_INPUT
            chunkLeft = 0
            return read(buffer, offset, length)
        }
        position += n
        chunkLeft -= n
        return n
    }

    override fun getUri(): Uri? = upstream.uri ?: spec?.uri

    override fun getResponseHeaders(): Map<String, List<String>> = upstream.responseHeaders

    override fun close() {
        if (open) upstream.close()
        open = false
        spec = null
    }

    class Factory(private val upstream: HttpDataSource.Factory) : DataSource.Factory {
        override fun createDataSource(): DataSource = ChunkedHttpDataSource(upstream.createDataSource())
    }

    companion object {
        /** Well under the ~10 MB above which googlevideo throttles; ~0.4 s each on a fast line. */
        const val CHUNK = 4L shl 20
    }
}

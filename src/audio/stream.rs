//! Bounded-memory HTTP stream exposed as blocking `Read + Seek` for rodio.
//!
//! The file is split into [`CHUNK`]-sized pieces fetched with `Range`
//! requests by a background task, which stays up to [`AHEAD`] chunks ahead of
//! the reader. At most [`MAX_CHUNKS`] are kept (~2 MiB), so memory doesn't
//! grow with track length. Seeking to a chunk that isn't cached fetches it on
//! demand; the reader blocks until it arrives.
//!
//! Media URLs can stop working mid-track: YouTube answers 403 past the
//! first ~1 MB for some clients/regions, and URLs expire. On 403/410 the
//! downloader asks [`Refresh`] for a new URL (the resolver re-resolves,
//! switching to JS-assisted resolution) and carries on at the same offset.

use std::{
    collections::HashMap,
    io::{self, Read, Seek, SeekFrom},
    sync::{Arc, Condvar, Mutex},
    time::Duration,
};

use futures_util::future::BoxFuture;
use reqwest::{
    StatusCode,
    header::{CONTENT_RANGE, HeaderMap, HeaderName, HeaderValue, RANGE},
};
use tokio::sync::Notify;

use super::extractor::AudioStream;

/// 256 KiB ≈ 16 s of 128 kbps audio per request.
const CHUNK: u64 = 256 * 1024;
/// Read-ahead: ~1 minute of audio.
const AHEAD: u64 = 4;
/// Hard cap on cached chunks (2 MiB).
const MAX_CHUNKS: usize = 8;
const RETRIES: u32 = 3;
/// New URLs fetched per track after 403/410 before giving up.
const REFRESHES: u32 = 2;

/// Produces a fresh media URL for the same track (`None`: can't).
pub type Refresh = Box<dyn Fn() -> BoxFuture<'static, Option<AudioStream>> + Send + Sync>;

#[derive(Debug, thiserror::Error)]
pub enum StreamError {
    /// 403/410 usually means the URL expired or needs JS-assisted resolution.
    #[error("media server returned HTTP {0}")]
    Status(StatusCode),
    #[error("media request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("media server sent no length")]
    UnknownLength,
}

impl StreamError {
    pub fn is_forbidden(&self) -> bool {
        matches!(self, Self::Status(s) if *s == StatusCode::FORBIDDEN || *s == StatusCode::GONE)
    }
}

struct State {
    chunks: HashMap<u64, Vec<u8>>,
    /// Chunk the reader is currently in; the downloader works ahead of it.
    reader_chunk: u64,
    /// Set when the reader is blocked on a missing chunk.
    wanted: Option<u64>,
    error: Option<String>,
    cancelled: bool,
}

struct Shared {
    state: Mutex<State>,
    /// Wakes the (blocking) reader when a chunk lands.
    arrived: Condvar,
    /// Wakes the (async) downloader when the reader moves or needs a chunk.
    moved: Notify,
    total: u64,
    chunk_count: u64,
}

impl Shared {
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

pub struct HttpStream {
    shared: Arc<Shared>,
    pos: u64,
}

impl HttpStream {
    /// Fetches the first chunk (so HTTP errors surface here, not mid-playback)
    /// and starts the background downloader.
    pub async fn open(
        http: &reqwest::Client,
        stream: &AudioStream,
        refresh: Option<Refresh>,
    ) -> Result<Self, StreamError> {
        let headers = header_map(stream);
        let (first, total) = fetch_chunk(http, &stream.url, &headers, 0, None).await?;
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                chunks: HashMap::from([(0, first)]),
                reader_chunk: 0,
                wanted: None,
                error: None,
                cancelled: false,
            }),
            arrived: Condvar::new(),
            moved: Notify::new(),
            total,
            chunk_count: total.div_ceil(CHUNK),
        });
        if shared.chunk_count > 1 {
            tokio::spawn(downloader(
                http.clone(),
                Source {
                    url: stream.url.clone(),
                    headers,
                },
                refresh,
                shared.clone(),
            ));
        }
        Ok(Self { shared, pos: 0 })
    }

    pub fn len(&self) -> u64 {
        self.shared.total
    }

    pub fn is_empty(&self) -> bool {
        self.shared.total == 0
    }
}

fn header_map(stream: &AudioStream) -> HeaderMap {
    stream
        .http_headers
        .iter()
        .filter_map(|(k, v)| {
            Some((
                HeaderName::from_bytes(k.as_bytes()).ok()?,
                HeaderValue::from_str(v).ok()?,
            ))
        })
        .collect()
}

/// Where chunks come from; replaced when the URL stops working.
struct Source {
    url: String,
    headers: HeaderMap,
}

/// Fetches chunk `index`; returns its bytes and the total file size.
async fn fetch_chunk(
    http: &reqwest::Client,
    url: &str,
    headers: &HeaderMap,
    index: u64,
    total: Option<u64>,
) -> Result<(Vec<u8>, u64), StreamError> {
    let start = index * CHUNK;
    let mut end = start + CHUNK - 1;
    if let Some(total) = total {
        end = end.min(total - 1);
    }
    let response = http
        .get(url)
        .headers(headers.clone())
        .header(RANGE, format!("bytes={start}-{end}"))
        .send()
        .await?;
    let total = match response.status() {
        StatusCode::PARTIAL_CONTENT => response
            .headers()
            .get(CONTENT_RANGE)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.rsplit('/').next())
            .and_then(|v| v.parse().ok())
            .ok_or(StreamError::UnknownLength)?,
        status => return Err(StreamError::Status(status)),
    };
    Ok((response.bytes().await?.to_vec(), total))
}

/// Next chunk to fetch: whatever the reader is blocked on, else the first
/// missing chunk in the read-ahead window.
fn next_chunk(state: &State, chunk_count: u64) -> Option<u64> {
    if let Some(w) = state.wanted.filter(|w| !state.chunks.contains_key(w)) {
        return Some(w);
    }
    (state.reader_chunk..(state.reader_chunk + AHEAD + 1).min(chunk_count))
        .find(|i| !state.chunks.contains_key(i))
}

/// Drops chunks farthest from the reader once over the cap, played ones first.
fn evict(state: &mut State) {
    while state.chunks.len() > MAX_CHUNKS {
        let reader = state.reader_chunk;
        let farthest = state.chunks.keys().copied().max_by_key(|&i| {
            if i < reader {
                (1, reader - i)
            } else {
                (0, i - reader)
            }
        });
        match farthest {
            Some(i) => state.chunks.remove(&i),
            None => break,
        };
    }
}

async fn downloader(
    http: reqwest::Client,
    mut source: Source,
    refresh: Option<Refresh>,
    shared: Arc<Shared>,
) {
    let mut refreshes = 0;
    loop {
        // Register interest before checking state, so a reader move between
        // the check and the await isn't missed.
        let moved = shared.moved.notified();
        let next = {
            let state = shared.lock();
            if state.cancelled {
                return;
            }
            next_chunk(&state, shared.chunk_count)
        };
        let Some(index) = next else {
            moved.await;
            continue;
        };

        let mut attempt = 0;
        let result = loop {
            match fetch_chunk(
                &http,
                &source.url,
                &source.headers,
                index,
                Some(shared.total),
            )
            .await
            {
                Ok((bytes, _)) if !bytes.is_empty() => break Ok(bytes),
                Ok(_) => break Err("empty chunk".to_owned()),
                // Retrying the same URL won't help: get a new one.
                Err(err) if err.is_forbidden() && refreshes < REFRESHES => {
                    refreshes += 1;
                    tracing::info!(%err, index, refreshes, "media URL rejected mid-track, re-resolving");
                    match new_source(refresh.as_ref(), &http, shared.total).await {
                        Some(fresh) => source = fresh,
                        None => break Err(err.to_string()),
                    }
                }
                Err(err) if attempt < RETRIES => {
                    attempt += 1;
                    tracing::debug!(%err, attempt, index, "chunk failed, retrying");
                    tokio::time::sleep(Duration::from_millis(500 * u64::from(attempt))).await;
                }
                Err(err) => break Err(err.to_string()),
            }
        };

        let mut state = shared.lock();
        match result {
            Ok(bytes) => {
                state.chunks.insert(index, bytes);
                if state.wanted == Some(index) {
                    state.wanted = None;
                }
                evict(&mut state);
            }
            Err(err) => {
                tracing::warn!(%err, index, "stream download failed");
                state.error = Some(err);
            }
        }
        let failed = state.error.is_some();
        drop(state);
        shared.arrived.notify_all();
        if failed {
            return;
        }
    }
}

/// A new URL for the same file (same length), or `None`.
async fn new_source(
    refresh: Option<&Refresh>,
    http: &reqwest::Client,
    total: u64,
) -> Option<Source> {
    let stream = refresh?().await?;
    let source = Source {
        url: stream.url.clone(),
        headers: header_map(&stream),
    };
    // A different format would have different bytes at the same offsets.
    match fetch_chunk(http, &source.url, &source.headers, 0, None).await {
        Ok((_, len)) if len == total => Some(source),
        Ok((_, len)) => {
            tracing::warn!(len, total, "re-resolved URL is a different file");
            None
        }
        Err(err) => {
            tracing::warn!(%err, "re-resolved URL fails too");
            None
        }
    }
}

impl Read for HttpStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.pos >= self.shared.total || buf.is_empty() {
            return Ok(0);
        }
        let index = self.pos / CHUNK;
        let mut state = self.shared.lock();
        if state.reader_chunk != index {
            state.reader_chunk = index;
            self.shared.moved.notify_one();
        }
        loop {
            if let Some(chunk) = state.chunks.get(&index) {
                let offset = (self.pos - index * CHUNK) as usize;
                let n = buf.len().min(chunk.len().saturating_sub(offset));
                if n == 0 {
                    return Ok(0);
                }
                buf[..n].copy_from_slice(&chunk[offset..offset + n]);
                self.pos += n as u64;
                return Ok(n);
            }
            if let Some(err) = &state.error {
                return Err(io::Error::other(err.clone()));
            }
            if state.wanted != Some(index) {
                state.wanted = Some(index);
                self.shared.moved.notify_one();
            }
            state = self
                .shared
                .arrived
                .wait(state)
                .unwrap_or_else(|e| e.into_inner());
        }
    }
}

impl Seek for HttpStream {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let target = match pos {
            SeekFrom::Start(n) => Some(n),
            SeekFrom::End(off) => self.shared.total.checked_add_signed(off),
            SeekFrom::Current(off) => self.pos.checked_add_signed(off),
        };
        self.pos = target
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "seek before start"))?;
        Ok(self.pos)
    }
}

impl Drop for HttpStream {
    fn drop(&mut self) {
        self.shared.lock().cancelled = true;
        self.shared.moved.notify_one();
        self.shared.arrived.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Serves `body` at `/ok` and `/limited`; `/limited` answers 403 past
    /// 1 MiB, like googlevideo does for some URLs.
    async fn media_server(body: Arc<Vec<u8>>) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    return;
                };
                let body = body.clone();
                tokio::spawn(async move {
                    let mut buf = vec![0; 4096];
                    let n = socket.read(&mut buf).await.unwrap_or(0);
                    let req = String::from_utf8_lossy(&buf[..n]).to_ascii_lowercase();
                    let range = req
                        .lines()
                        .find_map(|l| l.strip_prefix("range: bytes="))
                        .and_then(|r| r.trim().split_once('-'))
                        .map(|(a, b)| (a.parse::<usize>().unwrap(), b.parse::<usize>().unwrap()))
                        .unwrap();
                    let (start, end) = (range.0, range.1.min(body.len() - 1));
                    let head = if req.starts_with("get /limited") && start >= 1 << 20 {
                        "HTTP/1.1 403 Forbidden\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
                            .to_owned()
                    } else {
                        format!(
                            "HTTP/1.1 206 Partial Content\r\ncontent-range: bytes {start}-{end}/{}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                            body.len(),
                            end - start + 1
                        )
                    };
                    let _ = socket.write_all(head.as_bytes()).await;
                    if head.contains("206") {
                        let _ = socket.write_all(&body[start..=end]).await;
                    }
                });
            }
        });
        format!("http://{addr}")
    }

    fn audio(url: String) -> AudioStream {
        AudioStream {
            video_id: "dQw4w9WgXcQ".into(),
            title: None,
            url,
            format_id: None,
            ext: None,
            codec: None,
            bitrate_kbps: None,
            sample_rate: None,
            duration: None,
            content_length: None,
            http_headers: HashMap::new(),
            expires_at: None,
        }
    }

    async fn read_all(stream: HttpStream) -> io::Result<Vec<u8>> {
        tokio::task::spawn_blocking(move || {
            let mut stream = stream;
            let mut out = Vec::new();
            stream.read_to_end(&mut out).map(|_| out)
        })
        .await
        .unwrap()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn switches_url_on_403_mid_track() {
        let body: Arc<Vec<u8>> = Arc::new((0..1_600_000u32).map(|i| (i % 251) as u8).collect());
        let base = media_server(body.clone()).await;
        let http = reqwest::Client::new();
        let fresh = audio(format!("{base}/ok"));
        let refresh: Refresh = Box::new(move || {
            let fresh = fresh.clone();
            Box::pin(async move { Some(fresh) })
        });
        let stream = HttpStream::open(&http, &audio(format!("{base}/limited")), Some(refresh))
            .await
            .unwrap();
        assert_eq!(read_all(stream).await.unwrap(), *body);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn fails_on_403_without_refresh() {
        let body: Arc<Vec<u8>> = Arc::new(vec![7; 1_600_000]);
        let base = media_server(body).await;
        let http = reqwest::Client::new();
        let stream = HttpStream::open(&http, &audio(format!("{base}/limited")), None)
            .await
            .unwrap();
        assert!(read_all(stream).await.is_err());
    }

    fn state(reader: u64, have: &[u64]) -> State {
        State {
            chunks: have.iter().map(|&i| (i, vec![0])).collect(),
            reader_chunk: reader,
            wanted: None,
            error: None,
            cancelled: false,
        }
    }

    #[test]
    fn fetches_wanted_chunk_first() {
        let mut s = state(0, &[0, 1]);
        assert_eq!(next_chunk(&s, 100), Some(2));
        s.wanted = Some(50);
        assert_eq!(next_chunk(&s, 100), Some(50));
    }

    #[test]
    fn stops_at_window_and_file_end() {
        let s = state(0, &[0, 1, 2, 3, 4]);
        assert_eq!(
            next_chunk(&s, 100),
            None,
            "read-ahead window already cached"
        );
        let s = state(9, &[9]);
        assert_eq!(next_chunk(&s, 10), None, "no chunks past the end");
    }

    #[test]
    fn evicts_played_chunks_first_and_respects_cap() {
        let mut s = state(10, &[0, 1, 2, 3, 8, 9, 10, 11, 12, 13]);
        evict(&mut s);
        assert_eq!(s.chunks.len(), MAX_CHUNKS);
        assert!(!s.chunks.contains_key(&0) && !s.chunks.contains_key(&1));
        assert!(s.chunks.contains_key(&10) && s.chunks.contains_key(&13));
    }
}

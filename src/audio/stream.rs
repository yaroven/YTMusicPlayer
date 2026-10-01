//! Progressive HTTP download exposed as blocking `Read + Seek` for rodio.
//!
//! The file is fetched in 1 MiB `Range` chunks (googlevideo throttles
//! un-chunked downloads) into memory: ~1 MB per minute of 128 kbps AAC.
//! Reads past the downloaded part block until the bytes arrive.

use std::{
    io::{self, Read, Seek, SeekFrom},
    sync::{Arc, Condvar, Mutex},
    time::Duration,
};

use reqwest::{
    StatusCode,
    header::{CONTENT_RANGE, HeaderMap, HeaderName, HeaderValue, RANGE},
};

use super::extractor::AudioStream;

const CHUNK: u64 = 1 << 20;
const RETRIES: u32 = 3;

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
    data: Vec<u8>,
    total: u64,
    done: bool,
    error: Option<String>,
    cancelled: bool,
}

struct Shared {
    state: Mutex<State>,
    ready: Condvar,
}

impl Shared {
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

pub struct HttpStream {
    shared: Arc<Shared>,
    pos: u64,
    total: u64,
}

impl HttpStream {
    /// Fetches the first chunk (so HTTP errors surface here, not mid-playback)
    /// and continues downloading in a background task.
    pub async fn open(http: &reqwest::Client, stream: &AudioStream) -> Result<Self, StreamError> {
        let headers: HeaderMap = stream
            .http_headers
            .iter()
            .filter_map(|(k, v)| {
                Some((
                    HeaderName::from_bytes(k.as_bytes()).ok()?,
                    HeaderValue::from_str(v).ok()?,
                ))
            })
            .collect();

        let (first, total) = fetch_range(http, &stream.url, &headers, 0).await?;
        let done = first.len() as u64 >= total;
        let mut data = Vec::with_capacity(total as usize);
        data.extend_from_slice(&first);
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                data,
                total,
                done,
                error: None,
                cancelled: false,
            }),
            ready: Condvar::new(),
        });

        if !done {
            tokio::spawn(download_rest(
                http.clone(),
                stream.url.clone(),
                headers,
                shared.clone(),
                first.len() as u64,
            ));
        }
        Ok(Self {
            shared,
            pos: 0,
            total,
        })
    }

    pub fn len(&self) -> u64 {
        self.total
    }

    pub fn is_empty(&self) -> bool {
        self.total == 0
    }
}

async fn fetch_range(
    http: &reqwest::Client,
    url: &str,
    headers: &HeaderMap,
    start: u64,
) -> Result<(Vec<u8>, u64), StreamError> {
    let response = http
        .get(url)
        .headers(headers.clone())
        .header(RANGE, format!("bytes={start}-{}", start + CHUNK - 1))
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
        // Server ignored Range and sends the whole file.
        StatusCode::OK if start == 0 => response
            .content_length()
            .ok_or(StreamError::UnknownLength)?,
        status => return Err(StreamError::Status(status)),
    };
    Ok((response.bytes().await?.to_vec(), total))
}

async fn download_rest(
    http: reqwest::Client,
    url: String,
    headers: HeaderMap,
    shared: Arc<Shared>,
    mut offset: u64,
) {
    let total = shared.lock().total;
    while offset < total {
        if shared.lock().cancelled {
            return;
        }
        let mut attempt = 0;
        let chunk = loop {
            match fetch_range(&http, &url, &headers, offset).await {
                Ok((bytes, _)) if !bytes.is_empty() => break Ok(bytes),
                Ok(_) => break Err("empty chunk".to_owned()),
                Err(err) if attempt < RETRIES => {
                    attempt += 1;
                    tracing::debug!(%err, attempt, offset, "chunk failed, retrying");
                    tokio::time::sleep(Duration::from_millis(500 * u64::from(attempt))).await;
                }
                Err(err) => break Err(err.to_string()),
            }
        };
        let mut state = shared.lock();
        match chunk {
            Ok(bytes) => {
                offset += bytes.len() as u64;
                state.data.extend_from_slice(&bytes);
            }
            Err(err) => {
                tracing::warn!(%err, offset, "stream download failed");
                state.error = Some(err);
                state.done = true;
                shared.ready.notify_all();
                return;
            }
        }
        shared.ready.notify_all();
    }
    shared.lock().done = true;
    shared.ready.notify_all();
}

impl Read for HttpStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let mut state = self.shared.lock();
        loop {
            let len = state.data.len() as u64;
            if self.pos < len {
                let start = self.pos as usize;
                let n = buf.len().min((len - self.pos) as usize);
                buf[..n].copy_from_slice(&state.data[start..start + n]);
                self.pos += n as u64;
                return Ok(n);
            }
            if state.done || self.pos >= self.total {
                return match &state.error {
                    Some(err) if self.pos < self.total => Err(io::Error::other(err.clone())),
                    _ => Ok(0),
                };
            }
            state = self
                .shared
                .ready
                .wait(state)
                .unwrap_or_else(|e| e.into_inner());
        }
    }
}

impl Seek for HttpStream {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let target = match pos {
            SeekFrom::Start(n) => Some(n),
            SeekFrom::End(off) => self.total.checked_add_signed(off),
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
        self.shared.ready.notify_all();
    }
}

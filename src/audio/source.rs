//! Track source: turns a [`Track`] into a playable, bounded audio stream.
//!
//! It owns the whole Media URL lifecycle so callers don't have to: the URL
//! cache (memory + SQLite), one yt-dlp at a time, the fallback ladder (plain
//! → JS runtime → `yt-dlp -U`), the format check, a 403/410 when opening
//! (re-resolve once) or mid-track (fresh URL, same offset), prefetch of the
//! next track and the daily yt-dlp update. The duration falls back to the
//! library's when the media doesn't report one. Downloaded tracks play from
//! their file; loudness normalization uses YouTube's own measurement.
//!
//! yt-dlp sits behind the [`Extractor`] seam: [`YtDlp`] in the app, scripted
//! fakes in tests.

use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use anyhow::{Result, bail};
use futures_util::future::BoxFuture;

use super::{
    extractor::{self, AudioStream, YtDlp},
    js_runtime::JsRuntime,
    player::Load,
    resolver::{JsPolicy, StreamResolver},
    stream::{HttpStream, Media, Refresh},
};
use crate::{
    api::models::Track,
    catalog::Catalog,
    storage::{Download, Library},
};

/// Volume factor for a track `db` louder than YouTube's target (YouTube only
/// turns tracks down, never up).
pub fn gain_for(db: f32) -> f32 {
    if db > 0.0 {
        10f32.powf(-db / 20.0)
    } else {
        1.0
    }
}

/// What resolves video ids into media URLs (and keeps itself current).
pub trait Extractor: Send + Sync + 'static {
    /// Media URL for `video_id`; `js` when a JS runtime must be used.
    fn resolve<'a>(
        &'a self,
        video_id: &'a str,
        js: Option<&'a JsRuntime>,
    ) -> BoxFuture<'a, extractor::Result<AudioStream>>;

    /// Search without the Data API.
    fn search<'a>(
        &'a self,
        query: &'a str,
        max: u8,
    ) -> BoxFuture<'a, extractor::Result<Vec<Track>>>;

    /// Self-update now (after an extractor failure).
    fn update(&self) -> BoxFuture<'_, extractor::Result<String>>;

    /// Self-update if older than `max_age`; true if it updated.
    fn update_if_stale<'a>(
        &'a self,
        http: &'a reqwest::Client,
        max_age: Duration,
    ) -> BoxFuture<'a, extractor::Result<bool>>;

    /// Saves the track's audio (m4a) to `dest`.
    fn download<'a>(
        &'a self,
        video_id: &'a str,
        dest: &'a Path,
    ) -> BoxFuture<'a, extractor::Result<()>>;
}

impl Extractor for YtDlp {
    fn resolve<'a>(
        &'a self,
        video_id: &'a str,
        js: Option<&'a JsRuntime>,
    ) -> BoxFuture<'a, extractor::Result<AudioStream>> {
        Box::pin(async move {
            match js {
                Some(js) => self.resolve_with_js(video_id, js).await,
                None => YtDlp::resolve(self, video_id).await,
            }
        })
    }

    fn search<'a>(
        &'a self,
        query: &'a str,
        max: u8,
    ) -> BoxFuture<'a, extractor::Result<Vec<Track>>> {
        Box::pin(YtDlp::search(self, query, max))
    }

    fn update(&self) -> BoxFuture<'_, extractor::Result<String>> {
        Box::pin(YtDlp::update(self))
    }

    fn update_if_stale<'a>(
        &'a self,
        http: &'a reqwest::Client,
        max_age: Duration,
    ) -> BoxFuture<'a, extractor::Result<bool>> {
        Box::pin(YtDlp::update_if_stale(self, http, max_age))
    }

    fn download<'a>(
        &'a self,
        video_id: &'a str,
        dest: &'a Path,
    ) -> BoxFuture<'a, extractor::Result<()>> {
        Box::pin(YtDlp::download_audio(self, video_id, dest))
    }
}

/// An opened track, ready for the player.
pub struct Opened {
    pub media: Media,
    /// From the media, else from the library listing.
    pub duration: Option<Duration>,
    /// Loudness normalization factor (1.0: as is).
    pub gain: f32,
    /// The resolved stream (`None` for a downloaded file).
    pub stream: Option<AudioStream>,
}

impl Opened {
    /// What the player needs, tagged with `generation`.
    pub fn into_load(self, generation: u64) -> Load {
        Load {
            media: self.media,
            duration: self.duration,
            gain: self.gain,
            generation,
        }
    }
}

/// Optional parts of a [`TrackSource`].
#[derive(Clone, Default)]
pub struct SourceOptions {
    /// Persists resolved URLs and knows downloaded tracks.
    pub store: Option<Arc<Library>>,
    /// For loudness normalization (`None`: never).
    pub catalog: Option<Catalog>,
    /// Normalization on (shared by clones, so it can change while playing).
    pub normalize: Arc<AtomicBool>,
    /// Where downloads go.
    pub downloads: Option<PathBuf>,
}

#[derive(Clone)]
pub struct TrackSource {
    resolver: StreamResolver,
    extractor: Arc<dyn Extractor>,
    http: reqwest::Client,
    options: SourceOptions,
}

/// How long normalization may delay the start of a track.
const LOUDNESS_TIMEOUT: Duration = Duration::from_secs(3);

impl TrackSource {
    pub fn new(
        extractor: Arc<dyn Extractor>,
        http: reqwest::Client,
        bin_dir: PathBuf,
        policy: JsPolicy,
        options: SourceOptions,
    ) -> Self {
        Self {
            resolver: StreamResolver::new(
                extractor.clone(),
                http.clone(),
                bin_dir,
                policy,
                options.store.clone(),
            ),
            extractor,
            http,
            options,
        }
    }

    /// Opens `track`: the downloaded file if there is one, else resolves
    /// and starts streaming it. Errors are final for this attempt (the URL
    /// ladder and one re-resolve already ran).
    pub async fn open(&self, track: &Track) -> Result<Opened> {
        let listed = track.duration_secs.map(|s| Duration::from_secs(s.into()));
        let local = self
            .options
            .store
            .as_ref()
            .and_then(|s| s.download_path(&track.video_id).ok().flatten())
            .filter(|p| p.is_file());
        if let Some(path) = local {
            let gain = self.gain(&track.video_id).await;
            return Ok(Opened {
                media: Media::file(&path)?,
                duration: listed,
                gain,
                stream: None,
            });
        }
        let (opened, gain) = tokio::join!(self.stream(track), self.gain(&track.video_id));
        let (body, stream) = opened?;
        Ok(Opened {
            media: Media::Http(body),
            duration: stream.duration.or(listed),
            gain,
            stream: Some(stream),
        })
    }

    async fn stream(&self, track: &Track) -> Result<(HttpStream, AudioStream)> {
        let video_id = &*track.video_id;
        let stream = self.resolver.resolve(video_id).await?;
        if !stream.is_decodable() {
            bail!(
                "unsupported audio format {:?}/{:?}",
                stream.ext.as_deref().unwrap_or("?"),
                stream.codec.as_deref().unwrap_or("?")
            );
        }
        match HttpStream::open(&self.http, &stream, Some(self.refresher(video_id))).await {
            Ok(body) => Ok((body, stream)),
            Err(err) if err.is_forbidden() => {
                tracing::info!(video_id, %err, "media URL rejected, re-resolving");
                self.resolver.report_playback_failure(video_id);
                let stream = self.resolver.resolve(video_id).await?;
                let body =
                    HttpStream::open(&self.http, &stream, Some(self.refresher(video_id))).await?;
                Ok((body, stream))
            }
            Err(err) => Err(err.into()),
        }
    }

    /// Normalization factor: YouTube's own, which only turns loud tracks
    /// down. 1.0 when off, offline or slow.
    async fn gain(&self, video_id: &str) -> f32 {
        let Some(catalog) = &self.options.catalog else {
            return 1.0;
        };
        if !self.options.normalize.load(Ordering::Relaxed) {
            return 1.0;
        }
        match tokio::time::timeout(LOUDNESS_TIMEOUT, catalog.loudness_db(video_id)).await {
            Ok(Ok(Some(db))) => gain_for(db),
            Ok(Err(err)) => {
                tracing::debug!(%err, video_id, "loudness");
                1.0
            }
            _ => 1.0,
        }
    }

    /// Turns loudness normalization on or off (from the next track).
    pub fn set_normalize(&self, on: bool) {
        self.options.normalize.store(on, Ordering::Relaxed);
    }

    /// Saves `track`'s audio for offline play.
    pub async fn download(&self, track: &Track) -> Result<Download> {
        let (Some(dir), Some(store)) = (&self.options.downloads, &self.options.store) else {
            bail!("downloads aren't available here");
        };
        tokio::fs::create_dir_all(dir).await?;
        let path = dir.join(format!("{}.m4a", track.video_id));
        self.extractor.download(&track.video_id, &path).await?;
        let bytes = tokio::fs::metadata(&path).await?.len();
        let download = Download {
            track: track.clone(),
            path,
            bytes,
        };
        store.add_download(&download)?;
        Ok(download)
    }

    /// Deletes a downloaded track's file and record.
    pub async fn remove_download(&self, video_id: &str) -> Result<()> {
        let Some(store) = &self.options.store else {
            return Ok(());
        };
        if let Some(path) = store.download_path(video_id)? {
            let _ = tokio::fs::remove_file(&path).await;
        }
        store.remove_download(video_id)
    }

    /// The media URL alone (`ytm resolve`, for debugging).
    pub async fn resolve(&self, video_id: &str) -> Result<AudioStream> {
        Ok(self.resolver.resolve(video_id).await?)
    }

    /// Whether resolves currently go through a JS runtime.
    pub fn uses_js(&self) -> bool {
        self.resolver.uses_js()
    }

    /// Warms the URL cache for the next queued track.
    pub fn prefetch(&self, video_id: &str) {
        self.resolver.prefetch(video_id);
    }

    /// Searches YouTube without the Data API (shares the one-yt-dlp permit).
    pub async fn search(&self, query: &str, max: u8) -> Result<Vec<Track>> {
        Ok(self.resolver.search(query, max).await?)
    }

    /// Background daily self-update of the extractor.
    pub fn spawn_maintenance(&self) {
        self.resolver.spawn_maintenance();
    }

    /// New URLs for a track whose URL stopped working mid-playback: drops
    /// the cached one (and switches to JS-assisted resolution when allowed).
    fn refresher(&self, video_id: &str) -> Refresh {
        let (resolver, video_id) = (self.resolver.clone(), video_id.to_owned());
        Box::new(move || {
            let (resolver, video_id) = (resolver.clone(), video_id.clone());
            Box::pin(async move {
                resolver.report_playback_failure(&video_id);
                match resolver.resolve(&video_id).await {
                    Ok(stream) => Some(stream),
                    Err(err) => {
                        tracing::warn!(%err, video_id, "re-resolving failed");
                        None
                    }
                }
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        io::Read,
        sync::atomic::{AtomicUsize, Ordering},
    };

    use super::*;
    use crate::audio::extractor::ExtractorError;

    /// Serves `body` at `/ok` and `/limited` (403 past 1 MiB, as googlevideo
    /// does for some URLs) and 403 for everything at `/gone`.
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
                    let (start, end) = req
                        .lines()
                        .find_map(|l| l.strip_prefix("range: bytes="))
                        .and_then(|r| r.trim().split_once('-'))
                        .map(|(a, b)| (a.parse::<usize>().unwrap(), b.parse::<usize>().unwrap()))
                        .unwrap();
                    let end = end.min(body.len() - 1);
                    let forbidden = req.starts_with("get /gone")
                        || (req.starts_with("get /limited") && start >= 1 << 20);
                    let head = if forbidden {
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
                    if !forbidden {
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
            format_id: Some("140".into()),
            ext: Some("m4a".into()),
            codec: Some("mp4a.40.2".into()),
            bitrate_kbps: None,
            sample_rate: None,
            duration: None,
            content_length: None,
            http_headers: HashMap::new(),
            // Fresh for hours, like a googlevideo URL: cacheable.
            expires_at: Some(std::time::SystemTime::now() + Duration::from_secs(6 * 3600)),
        }
    }

    /// Hands out the given URLs in turn, counting resolves.
    struct Scripted {
        urls: Vec<String>,
        calls: AtomicUsize,
    }

    impl Extractor for Scripted {
        fn resolve<'a>(
            &'a self,
            _video_id: &'a str,
            _js: Option<&'a JsRuntime>,
        ) -> BoxFuture<'a, extractor::Result<AudioStream>> {
            let n = self.calls.fetch_add(1, Ordering::SeqCst);
            let url = self.urls.get(n).cloned();
            Box::pin(async move {
                url.map(audio)
                    .ok_or_else(|| ExtractorError::NoUrl("dQw4w9WgXcQ".into()))
            })
        }
        fn search<'a>(
            &'a self,
            _query: &'a str,
            _max: u8,
        ) -> BoxFuture<'a, extractor::Result<Vec<Track>>> {
            Box::pin(async { Ok(Vec::new()) })
        }
        fn update(&self) -> BoxFuture<'_, extractor::Result<String>> {
            Box::pin(async { Err(ExtractorError::NotManaged) })
        }
        fn update_if_stale<'a>(
            &'a self,
            _http: &'a reqwest::Client,
            _max_age: Duration,
        ) -> BoxFuture<'a, extractor::Result<bool>> {
            Box::pin(async { Ok(false) })
        }
        fn download<'a>(
            &'a self,
            _video_id: &'a str,
            _dest: &'a Path,
        ) -> BoxFuture<'a, extractor::Result<()>> {
            Box::pin(async { Err(ExtractorError::NotManaged) })
        }
    }

    fn source(urls: Vec<String>) -> (TrackSource, Arc<Scripted>) {
        let scripted = Arc::new(Scripted {
            urls,
            calls: AtomicUsize::new(0),
        });
        let source = TrackSource::new(
            scripted.clone(),
            reqwest::Client::new(),
            std::env::temp_dir(),
            JsPolicy::Never,
            SourceOptions::default(),
        );
        (source, scripted)
    }

    fn track(duration_secs: Option<u32>) -> Track {
        Track {
            video_id: "dQw4w9WgXcQ".into(),
            title: "t".into(),
            artist: "a".into(),
            duration_secs,
        }
    }

    async fn read_all(opened: Opened) -> std::io::Result<Vec<u8>> {
        tokio::task::spawn_blocking(move || {
            let mut body = opened.media;
            let mut out = Vec::new();
            body.read_to_end(&mut out).map(|_| out)
        })
        .await
        .unwrap()
    }

    fn body() -> Arc<Vec<u8>> {
        Arc::new((0..1_600_000u32).map(|i| (i % 251) as u8).collect())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn mid_track_403_gets_a_fresh_url() {
        let body = body();
        let base = media_server(body.clone()).await;
        let (source, scripted) = source(vec![format!("{base}/limited"), format!("{base}/ok")]);
        let opened = source.open(&track(Some(200))).await.unwrap();
        assert_eq!(
            opened.duration,
            Some(Duration::from_secs(200)),
            "listing duration"
        );
        assert_eq!(read_all(opened).await.unwrap(), *body);
        assert_eq!(scripted.calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn rejected_url_at_open_is_resolved_again() {
        let body = body();
        let base = media_server(body.clone()).await;
        let (source, scripted) = source(vec![format!("{base}/gone"), format!("{base}/ok")]);
        let opened = source.open(&track(None)).await.unwrap();
        assert_eq!(read_all(opened).await.unwrap(), *body);
        assert_eq!(scripted.calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn gives_up_when_every_url_fails() {
        let base = media_server(body()).await;
        let (source, _) = source(vec![format!("{base}/gone"), format!("{base}/gone")]);
        assert!(source.open(&track(None)).await.is_err());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn cached_url_is_reused() {
        let base = media_server(body()).await;
        let (source, scripted) = source(vec![format!("{base}/ok")]);
        // Fresh enough to cache: googlevideo URLs carry `expire`.
        let _ = source.resolve("dQw4w9WgXcQ").await;
        let _ = source.resolve("dQw4w9WgXcQ").await;
        assert_eq!(
            scripted.calls.load(Ordering::SeqCst),
            1,
            "in-flight/cached share one run"
        );
    }
}

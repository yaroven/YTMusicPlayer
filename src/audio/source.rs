//! Track source: turns a [`Track`] into a playable, bounded audio stream.
//!
//! It owns the whole Media URL lifecycle so callers don't have to: the URL
//! cache (memory + SQLite), the fallback ladder (InnerTube → yt-dlp plain →
//! JS runtime → `yt-dlp -U`, see `resolver`), one yt-dlp at a time, the
//! format check, a 403/410 when opening (re-resolve once) or mid-track
//! (fresh URL, same offset), prefetch of the next track and the daily
//! yt-dlp update. The duration falls back to the library's when the media
//! doesn't report one. Downloaded tracks play from their file; downloads
//! are fetched in ranges from the resolved URL, by yt-dlp only when that
//! isn't a plain file; loudness normalization uses YouTube's own
//! measurement.
//!
//! The URL sources sit behind seams: [`DirectResolver`] (InnerTube) and
//! [`Extractor`] ([`YtDlp`]), scripted fakes in tests.

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
    innertube::DirectResolver,
    js_runtime::JsRuntime,
    player::Load,
    resolver::{JsPolicy, StreamResolver},
    stream::{self, HttpStream, Media, Refresh},
};
use crate::{
    api::models::Track,
    catalog::{Catalog, MediaPair},
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

/// yt-dlp's side of the Track source: media URLs when InnerTube has none,
/// downloads that aren't plain files, search, cookies, its own updates.
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

    /// A browser's cookies as cookies.txt ("Import from browser").
    fn export_cookies<'a>(&'a self, browser: &'a str) -> BoxFuture<'a, extractor::Result<String>> {
        let _ = browser;
        Box::pin(async {
            Err(extractor::ExtractorError::Unavailable(
                "no cookie export".into(),
            ))
        })
    }
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

    fn export_cookies<'a>(&'a self, browser: &'a str) -> BoxFuture<'a, extractor::Result<String>> {
        Box::pin(YtDlp::export_cookies(self, browser))
    }
}

/// An opened track, ready for the player.
pub struct Opened {
    /// `None` when opened for a remote player (URL only).
    pub media: Option<Media>,
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
            url: self.stream.map(|s| s.url),
            track: None,
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
    /// Media URLs without yt-dlp, asked first (`None`: yt-dlp only).
    pub direct: Option<Arc<dyn DirectResolver>>,
    /// The browser age-restricted videos borrow a YouTube login from.
    pub cookies: extractor::Cookies,
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
    /// Songs and their clips found so far (video id → pair, and whether the
    /// clip of a song was looked for), see [`TrackSource::audio_id`].
    pairs: Arc<std::sync::Mutex<std::collections::HashMap<String, (MediaPair, bool)>>>,
}

/// Pairs kept in memory (a few hundred bytes each).
const PAIRS_KEEP: usize = 500;

/// HLS audio files kept in the temp dir (each ~3 MB).
const HLS_KEEP: usize = 3;

/// Deletes all but the `keep` newest files in `dir`.
fn prune_hls(dir: &std::path::Path, keep: usize) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<(std::time::SystemTime, std::path::PathBuf)> = entries
        .flatten()
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect();
    files.sort_by_key(|f| std::cmp::Reverse(f.0));
    for (_, path) in files.into_iter().skip(keep) {
        let _ = std::fs::remove_file(path);
    }
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
                options.direct.clone(),
                extractor.clone(),
                http.clone(),
                bin_dir,
                policy,
                options.store.clone(),
            ),
            extractor,
            http,
            options,
            pairs: Arc::default(),
        }
    }

    /// What plays as `track`'s sound: the studio song, also when the track
    /// is a clip (YouTube Music's "Song"), so a clip's intro or crowd noise
    /// isn't heard. One request the first time, then from memory; the
    /// track's own id when the catalog can't tell.
    async fn audio_id(&self, track: &Track) -> Arc<str> {
        match self.pair(track, false).await {
            Some(pair) => pair.song.into(),
            None => track.video_id.clone(),
        }
    }

    /// The track's song and clip (for the player's "Video"), looking for
    /// the clip of a song too.
    pub async fn media_pair(&self, track: &Track) -> Option<MediaPair> {
        self.pair(track, true).await
    }

    async fn pair(&self, track: &Track, find_clip: bool) -> Option<MediaPair> {
        let key = track.video_id.to_string();
        if let Some((pair, complete)) = self
            .pairs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&key)
            && (*complete || !find_clip || pair.video.is_some())
        {
            return Some(pair.clone());
        }
        let catalog = self.options.catalog.as_ref()?;
        let found = catalog
            .media_pair(&track.video_id, &track.title, &track.artist, find_clip)
            .await;
        match found {
            Ok(pair) => {
                let mut pairs = self.pairs.lock().unwrap_or_else(|e| e.into_inner());
                if pairs.len() >= PAIRS_KEEP {
                    pairs.clear();
                }
                pairs.insert(key, (pair.clone(), find_clip));
                Some(pair)
            }
            Err(err) => {
                tracing::debug!(%err, video_id = %track.video_id, "song / clip pair");
                None
            }
        }
    }

    /// Opens `track`: the downloaded file if there is one, else resolves
    /// and starts streaming it. Errors are final for this attempt (the URL
    /// ladder and one re-resolve already ran). `remote`: only resolve the
    /// URL, for a player that streams it itself (Chromecast).
    pub async fn open(&self, track: &Track, remote: bool) -> Result<Opened> {
        let listed = track.duration_secs.map(|s| Duration::from_secs(s.into()));
        if remote {
            let audio = self.audio_id(track).await;
            let stream = self.resolver.resolve(&audio).await?;
            return Ok(Opened {
                media: None,
                duration: stream.duration.or(listed),
                gain: 1.0,
                stream: Some(stream),
            });
        }
        let local = self
            .options
            .store
            .as_ref()
            .and_then(|s| s.download_path(&track.video_id).ok().flatten())
            .filter(|p| p.is_file());
        if let Some(path) = local {
            let gain = self.gain(&track.video_id).await;
            return Ok(Opened {
                media: Some(Media::file(&path)?),
                duration: listed,
                gain,
                stream: None,
            });
        }
        let audio = self.audio_id(track).await;
        if audio != track.video_id {
            tracing::info!(clip = %track.video_id, song = %audio, "playing the clip's studio song");
        }
        let (opened, gain) = tokio::join!(self.stream(&audio), self.gain(&audio));
        let (media, stream) = opened?;
        Ok(Opened {
            media: Some(media),
            duration: stream.duration.or(listed),
            gain,
            stream: Some(stream),
        })
    }

    async fn stream(&self, video_id: &str) -> Result<(Media, AudioStream)> {
        let stream = self.resolver.resolve(video_id).await?;
        if !stream.is_decodable() {
            bail!(
                "unsupported audio format {:?}/{:?}",
                stream.ext.as_deref().unwrap_or("?"),
                stream.codec.as_deref().unwrap_or("?")
            );
        }
        if stream.hls {
            return Ok((self.hls_audio(&stream).await?, stream));
        }
        match HttpStream::open(&self.http, &stream, Some(self.refresher(video_id))).await {
            Ok(body) => Ok((Media::Http(body), stream)),
            Err(err) if err.is_forbidden() => {
                tracing::info!(video_id, %err, "media URL rejected, re-resolving");
                self.resolver.report_playback_failure(video_id);
                let stream = self.resolver.resolve(video_id).await?;
                let body =
                    HttpStream::open(&self.http, &stream, Some(self.refresher(video_id))).await?;
                Ok((Media::Http(body), stream))
            }
            Err(err) => Err(err.into()),
        }
    }

    /// An HLS track (age-restricted, signed in): its audio fetched into a
    /// temp file first (~1 s), then played and seeked like a download. The
    /// last few stay, for replays and the preloaded next track.
    async fn hls_audio(&self, stream: &AudioStream) -> Result<Media> {
        let dir = std::env::temp_dir().join("yarek-music-hls");
        tokio::fs::create_dir_all(&dir).await?;
        let path = dir.join(format!("{}.aac", stream.video_id));
        if !path.is_file() {
            let started = std::time::Instant::now();
            let headers: Vec<(String, String)> = stream
                .http_headers
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            let bytes = super::hls::download_audio(&self.http, &stream.url, &headers, &path)
                .await
                .map_err(|err| anyhow::anyhow!("age-restricted song (HLS): {err:#}"))?;
            tracing::info!(video_id = %stream.video_id, bytes, elapsed = ?started.elapsed(), "HLS audio");
            prune_hls(&dir, HLS_KEEP);
        }
        Ok(Media::file(&path)?)
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

    /// Which browser's cookies age-restricted videos use ("" = none).
    pub fn set_cookies_browser(&self, browser: &str) {
        self.options.cookies.set(browser);
    }

    /// `browser`'s youtube.com sign-in (cookies.txt text).
    pub async fn export_cookies(&self, browser: &str) -> Result<String> {
        Ok(self.extractor.export_cookies(browser).await?)
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
        // The studio song's sound, also for a clip (as it plays) — and a
        // clip whose song can't be told now waits for a later try rather
        // than keeping its own sound.
        let audio = self.song_id(track).await?;
        self.fetch_audio(&audio, &path).await?;
        let bytes = tokio::fs::metadata(&path).await?.len();
        let download = Download {
            track: track.clone(),
            path,
            bytes,
        };
        store.add_download(&download)?;
        store.set_download_audio(&track.video_id, &audio)?;
        Ok(download)
    }

    /// Saves `video_id`'s audio to `dest`: from its resolved URL in ranges
    /// when that is a plain m4a file (no second lookup, no process), else —
    /// or when that fails other than for the network — through yt-dlp.
    async fn fetch_audio(&self, video_id: &str, dest: &Path) -> Result<()> {
        let stream = match self.resolver.resolve(video_id).await {
            Ok(stream) => Some(stream),
            Err(err) if crate::net::is_network(&err) => return Err(err),
            Err(err) => {
                tracing::info!(video_id, %err, "no URL to download from; yt-dlp downloads it");
                None
            }
        };
        if let Some(stream) = stream.filter(AudioStream::is_plain_file) {
            match stream::download_file(&self.http, &stream, dest).await {
                Ok(bytes) => {
                    tracing::info!(video_id, bytes, via = ?stream.via, "downloaded");
                    return Ok(());
                }
                Err(stream::StreamError::Http(err)) if err.is_connect() || err.is_timeout() => {
                    return Err(err.into());
                }
                Err(err) => {
                    tracing::info!(video_id, %err, "ranged download failed; yt-dlp downloads it");
                    self.resolver.report_playback_failure(video_id);
                }
            }
        }
        Ok(self.extractor.download(video_id, dest).await?)
    }

    /// The studio song of `track` (itself for a song), or an error when the
    /// catalog can't be asked right now.
    async fn song_id(&self, track: &Track) -> Result<Arc<str>> {
        if self.options.catalog.is_none() {
            return Ok(track.video_id.clone());
        }
        match self.pair(track, false).await {
            Some(pair) => Ok(pair.song.into()),
            None => bail!("couldn't ask YouTube Music which song this is"),
        }
    }

    /// A download from before the player recorded whose sound it holds:
    /// a clip downloaded with its own sound gets the studio song's (to a
    /// new file first, so the old one plays until then). True when it was
    /// replaced.
    pub async fn recheck_download(&self, track: &Track) -> Result<bool> {
        let (Some(dir), Some(store)) = (&self.options.downloads, &self.options.store) else {
            return Ok(false);
        };
        let Some(path) = store.download_path(&track.video_id)? else {
            return Ok(false);
        };
        let audio = self.song_id(track).await?;
        if audio != track.video_id {
            let fresh = dir.join(format!("{}.song.m4a", track.video_id));
            let _ = tokio::fs::remove_file(&fresh).await;
            if let Err(err) = self.fetch_audio(&audio, &fresh).await {
                let _ = tokio::fs::remove_file(&fresh).await;
                return Err(err);
            }
            tokio::fs::rename(&fresh, &path).await?;
            let bytes = tokio::fs::metadata(&path).await?.len();
            store.add_download(&Download {
                track: track.clone(),
                path,
                bytes,
            })?;
            tracing::info!(clip = %track.video_id, song = %audio, "download now has the studio song's sound");
        }
        store.set_download_audio(&track.video_id, &audio)?;
        Ok(audio != track.video_id)
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

    /// Deletes every downloaded track; returns how many went.
    pub async fn remove_all_downloads(&self) -> Result<usize> {
        let Some(store) = &self.options.store else {
            return Ok(0);
        };
        let downloads = store.downloads()?;
        for download in &downloads {
            let _ = tokio::fs::remove_file(&download.path).await;
        }
        store.clear_downloads()?;
        Ok(downloads.len())
    }

    /// Deletes files in the downloads folder that no record points at: what
    /// a cancelled download leaves behind (yt-dlp with `--no-part` and the
    /// ranged download both write straight to the final file).
    pub async fn clean_downloads(&self) -> Result<usize> {
        let (Some(dir), Some(store)) = (&self.options.downloads, &self.options.store) else {
            return Ok(0);
        };
        let kept: std::collections::HashSet<PathBuf> =
            store.downloads()?.into_iter().map(|d| d.path).collect();
        let mut entries = match tokio::fs::read_dir(dir).await {
            Ok(entries) => entries,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(0),
            Err(err) => return Err(err.into()),
        };
        let mut removed = 0;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if !kept.contains(&path) && tokio::fs::remove_file(&path).await.is_ok() {
                removed += 1;
            }
        }
        Ok(removed)
    }

    /// The media URL alone (`yarek resolve`, for debugging).
    pub async fn resolve(&self, video_id: &str) -> Result<AudioStream> {
        self.resolver.resolve(video_id).await
    }

    /// Whether resolves currently go through a JS runtime.
    pub fn uses_js(&self) -> bool {
        self.resolver.uses_js()
    }

    /// Warms the URL cache (and the song / clip pair) for the next queued
    /// track.
    pub fn prefetch(&self, track: &Track) {
        let source = self.clone();
        let track = track.clone();
        tokio::spawn(async move {
            let audio = source.audio_id(&track).await;
            source.resolver.prefetch(&audio);
        });
    }

    /// Searches YouTube without the Data API (shares the one-yt-dlp permit).
    pub async fn search(&self, query: &str, max: u8) -> Result<Vec<Track>> {
        Ok(self.resolver.search(query, max).await?)
    }

    /// Background daily self-update of the extractor and InnerTube's client list.
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
    use crate::audio::{extractor::ExtractorError, innertube::DirectError};

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
            hls: false,
            via: crate::audio::media_url::Via::YtDlp,
        }
    }

    /// What the fake InnerTube answers, in turn.
    #[derive(Clone)]
    enum Answer {
        Url(String),
        Refuse,
        Offline,
    }

    /// A scripted InnerTube: one [`Answer`] per resolve (the last repeats).
    struct Direct {
        answers: Vec<Answer>,
        calls: AtomicUsize,
        /// Served as the file's length (downloads need it).
        length: Option<u64>,
    }

    impl Direct {
        fn new(answers: Vec<Answer>) -> Arc<Self> {
            Arc::new(Self {
                answers,
                calls: AtomicUsize::new(0),
                length: None,
            })
        }
    }

    impl DirectResolver for Direct {
        fn resolve<'a>(
            &'a self,
            video_id: &'a str,
        ) -> BoxFuture<'a, Result<AudioStream, DirectError>> {
            let n = self.calls.fetch_add(1, Ordering::SeqCst);
            let answer = self.answers[n.min(self.answers.len() - 1)].clone();
            Box::pin(async move {
                match answer {
                    Answer::Url(url) => Ok(AudioStream {
                        video_id: video_id.to_owned(),
                        content_length: self.length,
                        via: crate::audio::media_url::Via::InnerTube,
                        ..audio(url)
                    }),
                    Answer::Refuse => Err(DirectError::Refused(
                        "LOGIN_REQUIRED: Sign in to confirm your age".into(),
                    )),
                    Answer::Offline => {
                        let err = reqwest::Client::new()
                            .get("http://127.0.0.1:9/")
                            .send()
                            .await
                            .unwrap_err();
                        Err(DirectError::from(err))
                    }
                }
            })
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
        source_with(urls, None)
    }

    /// yt-dlp handing out `urls`, behind `direct` when given.
    fn source_with(urls: Vec<String>, direct: Option<Arc<Direct>>) -> (TrackSource, Arc<Scripted>) {
        let scripted = Arc::new(Scripted {
            urls,
            calls: AtomicUsize::new(0),
        });
        let options = SourceOptions {
            direct: direct.map(|d| d as Arc<dyn DirectResolver>),
            ..SourceOptions::default()
        };
        let source = TrackSource::new(
            scripted.clone(),
            reqwest::Client::new(),
            std::env::temp_dir(),
            JsPolicy::Never,
            options,
        );
        (source, scripted)
    }

    /// A source with a real downloads folder and library behind it.
    fn downloading_source(dir: &Path) -> (TrackSource, Arc<crate::storage::Library>) {
        let store = Arc::new(crate::storage::Library::open_in_memory().unwrap());
        let options = SourceOptions {
            downloads: Some(dir.to_path_buf()),
            store: Some(store.clone()),
            ..SourceOptions::default()
        };
        let source = TrackSource::new(
            Arc::new(Scripted {
                urls: Vec::new(),
                calls: AtomicUsize::new(0),
            }),
            reqwest::Client::new(),
            std::env::temp_dir(),
            JsPolicy::Never,
            options,
        );
        (source, store)
    }

    fn track(duration_secs: Option<u32>) -> Track {
        track_id("dQw4w9WgXcQ", duration_secs)
    }

    fn track_id(id: &str, duration_secs: Option<u32>) -> Track {
        Track {
            video_id: id.into(),
            title: "t".into(),
            artist: "a".into(),
            duration_secs,
            explicit: false,
        }
    }

    async fn read_all(opened: Opened) -> std::io::Result<Vec<u8>> {
        tokio::task::spawn_blocking(move || {
            let mut body = opened.media.expect("opened locally");
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
        let opened = source.open(&track(Some(200)), false).await.unwrap();
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
        let opened = source.open(&track(None), false).await.unwrap();
        assert_eq!(read_all(opened).await.unwrap(), *body);
        assert_eq!(scripted.calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn gives_up_when_every_url_fails() {
        let base = media_server(body()).await;
        let (source, _) = source(vec![format!("{base}/gone"), format!("{base}/gone")]);
        assert!(source.open(&track(None), false).await.is_err());
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

    #[tokio::test(flavor = "multi_thread")]
    async fn innertube_url_needs_no_ytdlp() {
        let body = body();
        let base = media_server(body.clone()).await;
        let direct = Direct::new(vec![Answer::Url(format!("{base}/ok"))]);
        let (source, ytdlp) = source_with(vec![], Some(direct.clone()));
        let opened = source.open(&track(None), false).await.unwrap();
        assert_eq!(read_all(opened).await.unwrap(), *body);
        assert_eq!(direct.calls.load(Ordering::SeqCst), 1);
        assert_eq!(ytdlp.calls.load(Ordering::SeqCst), 0, "no yt-dlp run");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn refused_by_innertube_asks_ytdlp() {
        let body = body();
        let base = media_server(body.clone()).await;
        let direct = Direct::new(vec![Answer::Refuse]);
        let (source, ytdlp) = source_with(vec![format!("{base}/ok")], Some(direct.clone()));
        let opened = source.open(&track(None), false).await.unwrap();
        assert_eq!(read_all(opened).await.unwrap(), *body);
        assert_eq!(ytdlp.calls.load(Ordering::SeqCst), 1);
    }

    /// Offline, yt-dlp would fail the same way (seconds later, ~90 MB):
    /// the error goes to the session, which waits for the connection.
    #[tokio::test(flavor = "multi_thread")]
    async fn offline_does_not_start_ytdlp() {
        let direct = Direct::new(vec![Answer::Offline]);
        let (source, ytdlp) = source_with(vec!["http://unused".into()], Some(direct));
        let Err(err) = source.open(&track(None), false).await else {
            panic!("opened while offline");
        };
        assert!(crate::net::is_network(&err), "{err:#}");
        assert_eq!(ytdlp.calls.load(Ordering::SeqCst), 0);
    }

    /// The media server refusing an InnerTube URL sends the song to yt-dlp;
    /// after a few such songs InnerTube is left out for the run.
    #[tokio::test(flavor = "multi_thread")]
    async fn rejected_innertube_urls_go_to_ytdlp() {
        let body = body();
        let base = media_server(body.clone()).await;
        let direct = Direct::new(vec![Answer::Url(format!("{base}/gone"))]);
        let ok = format!("{base}/ok");
        let (source, ytdlp) = source_with(vec![ok.clone(); 4], Some(direct.clone()));
        for (n, id) in ["aaaaaaaaaaa", "bbbbbbbbbbb", "ccccccccccc"]
            .iter()
            .enumerate()
        {
            let opened = source.open(&track_id(id, None), false).await.unwrap();
            assert_eq!(read_all(opened).await.unwrap(), *body);
            assert_eq!(direct.calls.load(Ordering::SeqCst), n + 1);
            assert_eq!(ytdlp.calls.load(Ordering::SeqCst), n + 1);
        }
        // Three songs refused: the fourth doesn't ask InnerTube at all.
        let opened = source
            .open(&track_id("ddddddddddd", None), false)
            .await
            .unwrap();
        assert_eq!(read_all(opened).await.unwrap(), *body);
        assert_eq!(direct.calls.load(Ordering::SeqCst), 3);
        assert_eq!(ytdlp.calls.load(Ordering::SeqCst), 4);
    }

    /// A plain URL is downloaded in ranges by us: yt-dlp (whose download
    /// fails in this fake) isn't started.
    #[tokio::test(flavor = "multi_thread")]
    async fn downloads_plain_files_in_ranges() {
        let body = Arc::new(
            (0..2_500_000u32)
                .map(|i| (i % 253) as u8)
                .collect::<Vec<_>>(),
        );
        let base = media_server(body.clone()).await;
        let dir = std::env::temp_dir().join(format!("yarek-ranged-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = Arc::new(crate::storage::Library::open_in_memory().unwrap());
        let direct = Arc::new(Direct {
            answers: vec![Answer::Url(format!("{base}/ok"))],
            calls: AtomicUsize::new(0),
            length: Some(body.len() as u64),
        });
        let options = SourceOptions {
            downloads: Some(dir.clone()),
            store: Some(store.clone()),
            direct: Some(direct),
            ..SourceOptions::default()
        };
        let scripted = Arc::new(Scripted {
            urls: Vec::new(),
            calls: AtomicUsize::new(0),
        });
        let source = TrackSource::new(
            scripted,
            reqwest::Client::new(),
            std::env::temp_dir(),
            JsPolicy::Never,
            options,
        );
        let download = source.download(&track(Some(213))).await.unwrap();
        assert_eq!(download.bytes, body.len() as u64);
        assert_eq!(std::fs::read(&download.path).unwrap(), *body);
        assert_eq!(store.downloads().unwrap().len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Stopping a download leaves a half-written file (downloads write
    /// straight to it): the next cleanup throws it away, and "delete all"
    /// takes the finished ones with their records.
    #[tokio::test]
    async fn cleans_leftovers_and_deletes_every_download() {
        let dir = std::env::temp_dir().join(format!("yarek-downloads-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (source, store) = downloading_source(&dir);

        let done = dir.join("dQw4w9WgXcQ.m4a");
        let half = dir.join("YkLLcIKhJ64.m4a");
        std::fs::write(&done, b"audio").unwrap();
        std::fs::write(&half, b"half").unwrap();
        store
            .add_download(&Download {
                track: track(Some(213)),
                path: done.clone(),
                bytes: 5,
            })
            .unwrap();

        assert_eq!(source.clean_downloads().await.unwrap(), 1);
        assert!(done.exists() && !half.exists(), "only the leftover goes");

        assert_eq!(source.remove_all_downloads().await.unwrap(), 1);
        assert!(!done.exists());
        assert!(store.downloads().unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

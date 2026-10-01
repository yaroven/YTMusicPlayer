//! Caching, deduplicating front-end over [`YtDlp`].
//!
//! - Resolved URLs are reused until shortly before they expire (~6 h).
//! - Concurrent requests for the same id (play + prefetch) share one yt-dlp run.
//! - At most [`MAX_CONCURRENT`] yt-dlp processes run at once.
//! - Fallback ladder on failure: plain → with JS runtime → after `yt-dlp -U`.
//!
//! - Resolved URLs are also persisted in the library DB, so a restart doesn't
//!   pay the ~6 s yt-dlp run again; memory keeps only [`MEMORY_CAP`] entries.

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use tokio::sync::{OnceCell, Semaphore};

use crate::storage::Library;

use super::{
    extractor::{AudioStream, ExtractorError, Result, YtDlp},
    js_runtime::JsRuntime,
};

/// Keeps a few yt-dlp processes (~100 MB each while running) from piling up,
/// and avoids tripping YouTube's rate limiting.
const MAX_CONCURRENT: usize = 2;
/// A URL this close to expiry is re-resolved rather than handed to the player.
const FRESH_MARGIN: Duration = Duration::from_secs(30 * 60);
const UPDATE_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
/// In-memory entries (~2 KB each); older ones are served from the DB.
const MEMORY_CAP: usize = 16;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum JsPolicy {
    /// Never use a JS runtime.
    Never,
    /// Only after a plain resolve fails; then stick with it for the session.
    #[default]
    OnDemand,
    /// Always pass a JS runtime.
    Always,
}

#[derive(Clone)]
pub struct StreamResolver {
    inner: Arc<Inner>,
}

struct Inner {
    ytdlp: YtDlp,
    http: reqwest::Client,
    bin_dir: PathBuf,
    policy: JsPolicy,
    /// Set once a plain resolve or plain URL has failed: from then on, every
    /// resolve goes straight to JS instead of paying for two attempts.
    js_required: AtomicBool,
    js: OnceCell<JsRuntime>,
    update_tried: AtomicBool,
    permits: Semaphore,
    cache: Mutex<HashMap<String, Arc<OnceCell<AudioStream>>>>,
    store: Option<Arc<Library>>,
}

impl StreamResolver {
    pub fn new(ytdlp: YtDlp, http: reqwest::Client, bin_dir: PathBuf, policy: JsPolicy) -> Self {
        Self::with_store(ytdlp, http, bin_dir, policy, None)
    }

    /// Like [`new`](Self::new), persisting resolved URLs in `store`.
    pub fn with_store(
        ytdlp: YtDlp,
        http: reqwest::Client,
        bin_dir: PathBuf,
        policy: JsPolicy,
        store: Option<Arc<Library>>,
    ) -> Self {
        if let Some(store) = &store
            && let Err(err) = store.purge_expired_streams(unix_now())
        {
            tracing::warn!(%err, "purging stream cache");
        }
        Self {
            inner: Arc::new(Inner {
                ytdlp,
                http,
                bin_dir,
                policy,
                js_required: AtomicBool::new(policy == JsPolicy::Always),
                js: OnceCell::new(),
                update_tried: AtomicBool::new(false),
                permits: Semaphore::new(MAX_CONCURRENT),
                cache: Mutex::new(HashMap::new()),
                store,
            }),
        }
    }

    pub fn ytdlp(&self) -> &YtDlp {
        &self.inner.ytdlp
    }

    /// Whether resolves currently go through a JS runtime.
    pub fn uses_js(&self) -> bool {
        self.inner.js_required.load(Ordering::Relaxed)
    }

    /// Returns a fresh stream for `video_id`, from cache when possible.
    pub async fn resolve(&self, video_id: &str) -> Result<AudioStream> {
        let cell = {
            let mut cache = self.inner.cache.lock().unwrap_or_else(|e| e.into_inner());
            match cache.get(video_id) {
                // In flight (empty) or still fresh: share it.
                Some(cell) if cell.get().is_none_or(|s| s.is_fresh(FRESH_MARGIN)) => cell.clone(),
                _ => {
                    if cache.len() >= MEMORY_CAP {
                        // Keep in-flight entries; resolved ones live on in the DB.
                        cache.retain(|_, cell| cell.get().is_none());
                    }
                    let cell = Arc::new(OnceCell::new());
                    cache.insert(video_id.to_owned(), cell.clone());
                    cell
                }
            }
        };
        // Errors are not stored in the cell, so the next call retries.
        cell.get_or_try_init(|| async {
            if let Some(stream) = self.load_persisted(video_id) {
                return Ok(stream);
            }
            let stream = self.resolve_uncached(video_id).await?;
            self.persist(&stream);
            Ok(stream)
        })
        .await
        .cloned()
    }

    fn load_persisted(&self, video_id: &str) -> Option<AudioStream> {
        let store = self.inner.store.as_ref()?;
        let min_expiry = unix_now() + FRESH_MARGIN.as_secs() as i64;
        let json = store.cached_stream(video_id, min_expiry).ok()??;
        serde_json::from_str(&json).ok()
    }

    fn persist(&self, stream: &AudioStream) {
        let (Some(store), Some(expires)) = (&self.inner.store, stream.expires_at) else {
            return;
        };
        let expires = expires
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64);
        let result = serde_json::to_string(stream)
            .map_err(anyhow::Error::from)
            .and_then(|json| store.put_stream(&stream.video_id, &json, expires));
        if let Err(err) = result {
            tracing::warn!(%err, "persisting stream URL");
        }
    }

    /// Warms the cache for the next queued track; errors are only logged.
    pub fn prefetch(&self, video_id: &str) {
        let this = self.clone();
        let id = video_id.to_owned();
        tokio::spawn(async move {
            if let Err(err) = this.resolve(&id).await {
                tracing::debug!(video_id = %id, %err, "prefetch failed");
            }
        });
    }

    /// The player calls this when a resolved URL returns 403/410: drops the
    /// cached URL and, under [`JsPolicy::OnDemand`], switches to JS resolution.
    pub fn report_playback_failure(&self, video_id: &str) {
        self.inner
            .cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(video_id);
        if let Some(store) = &self.inner.store {
            let _ = store.put_stream(video_id, "{}", 0); // expire it
        }
        if self.inner.policy == JsPolicy::OnDemand
            && !self.inner.js_required.swap(true, Ordering::Relaxed)
        {
            tracing::info!("playback URL rejected; switching to JS-assisted resolution");
        }
    }

    /// Daily `yt-dlp -U` for the managed binary. Run in the background at startup.
    pub fn spawn_maintenance(&self) {
        let this = self.clone();
        tokio::spawn(async move {
            match this.inner.ytdlp.update_if_stale(UPDATE_INTERVAL).await {
                Ok(true) => tracing::info!("yt-dlp updated"),
                Ok(false) => {}
                Err(err) => tracing::warn!(%err, "yt-dlp update failed"),
            }
        });
    }

    async fn resolve_uncached(&self, video_id: &str) -> Result<AudioStream> {
        let _permit = self
            .inner
            .permits
            .acquire()
            .await
            .expect("semaphore never closed");

        let mut use_js = self.uses_js();
        let mut result = self.attempt(video_id, use_js).await;

        if !use_js && self.inner.policy == JsPolicy::OnDemand && is_retryable(&result) {
            tracing::info!(video_id, "plain resolve failed; retrying with JS runtime");
            use_js = true;
            result = self.attempt(video_id, true).await;
            if result.is_ok() {
                self.inner.js_required.store(true, Ordering::Relaxed);
            }
        }

        // An extractor broken by a YouTube change is fixed by updating yt-dlp.
        // An outdated managed copy is fixed the same way.
        let outdated = matches!(result, Err(ExtractorError::Outdated { .. }));
        if (is_retryable(&result) || outdated) && self.try_update_once().await {
            result = self.attempt(video_id, use_js).await;
        }
        result
    }

    async fn attempt(&self, video_id: &str, use_js: bool) -> Result<AudioStream> {
        if use_js {
            let js = self.js_runtime().await?;
            self.inner.ytdlp.resolve_with_js(video_id, js).await
        } else {
            self.inner.ytdlp.resolve(video_id).await
        }
    }

    /// Locates a JS runtime, or downloads QuickJS-NG once, on first need.
    async fn js_runtime(&self) -> Result<&JsRuntime> {
        let inner = &self.inner;
        inner
            .js
            .get_or_try_init(|| async {
                if let Some(rt) = JsRuntime::locate(&inner.bin_dir) {
                    tracing::info!(runtime = ?rt.kind, path = %rt.path.display(), "using JS runtime");
                    return Ok(rt);
                }
                JsRuntime::download_quickjs(&inner.http, &inner.bin_dir)
                    .await?
                    .ok_or(ExtractorError::NoJsRuntime)
            })
            .await
    }

    /// Runs `yt-dlp -U` at most once per session; true if it succeeded.
    async fn try_update_once(&self) -> bool {
        if self.inner.update_tried.swap(true, Ordering::Relaxed) {
            return false;
        }
        match self.inner.ytdlp.update().await {
            Ok(out) => {
                tracing::info!(output = %out.trim(), "yt-dlp self-update");
                true
            }
            Err(ExtractorError::NotManaged) => false,
            Err(err) => {
                tracing::warn!(%err, "yt-dlp self-update failed");
                false
            }
        }
    }
}

/// Failures that a JS runtime or a newer yt-dlp may fix. Unavailable videos,
/// bad ids, timeouts and spawn errors are final.
fn is_retryable(result: &Result<AudioStream>) -> bool {
    matches!(
        result,
        Err(ExtractorError::Failed { .. } | ExtractorError::NoUrl(_))
    )
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

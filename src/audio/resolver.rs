//! Caching, deduplicating front-end over [`YtDlp`].
//!
//! - Resolved URLs are reused until shortly before they expire (~6 h).
//! - Concurrent requests for the same id (play + prefetch) share one yt-dlp run.
//! - At most [`MAX_CONCURRENT`] yt-dlp processes run at once.
//! - Fallback ladder on failure: plain → with JS runtime → after `yt-dlp -U`.
//!
//! TODO: persist the cache in `storage` so URLs survive restarts.

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
}

impl StreamResolver {
    pub fn new(ytdlp: YtDlp, http: reqwest::Client, bin_dir: PathBuf, policy: JsPolicy) -> Self {
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
                    let cell = Arc::new(OnceCell::new());
                    cache.insert(video_id.to_owned(), cell.clone());
                    cell
                }
            }
        };
        // Errors are not stored in the cell, so the next call retries.
        cell.get_or_try_init(|| self.resolve_uncached(video_id))
            .await
            .cloned()
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
        if is_retryable(&result) && self.try_update_once().await {
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

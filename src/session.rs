//! UI-independent player core shared by the TUI and the GUI: library access,
//! queue and playback, background work (resolve, sync, likes, playlist edits)
//! and media-key handling. Frontends call the action methods and await
//! [`Session::next_event`] to learn what changed.

use std::{
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::Result;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

use crate::{
    account::{Account, Flow, Prompt},
    api::{
        client::YouTubeClient,
        models::{LIKED_PLAYLIST_ID, Playlist, Track},
    },
    audio::{
        Opened, TrackSource,
        player::{PlayState, Playback, PlayerEvent, PlayerHandle, PlayerStatus},
        queue::{Queue, Repeat},
    },
    media::{MediaAction, MediaControls},
    storage::Library,
    sync::{SyncReport, sync_library},
    sysmem,
};

pub const SEEK_STEP_SECS: i64 = 5;
pub const VOLUME_STEP: f32 = 0.05;
/// Stop auto-skipping after this many tracks in a row fail to load.
const MAX_CONSECUTIVE_FAILURES: u32 = 3;
/// How often memory use is re-measured for display.
const MEMORY_SAMPLE: Duration = Duration::from_secs(2);
const NOT_SIGNED_IN: &str = "Not signed in: “Sign in” in the window, or `ytm login`";
/// Results per online search (one API page).
const SEARCH_RESULTS: u8 = 25;
/// Smallest memory change worth a redraw.
const MEMORY_STEP: u64 = 2 << 20;

pub struct Deps {
    pub library: Arc<Library>,
    pub source: TrackSource,
    pub http: reqwest::Client,
    pub liked_music_only: bool,
    pub volume: f32,
    pub media_controls: bool,
    pub audio_device: Option<String>,
    /// OAuth client, sign-in and the YouTube API client.
    pub account: Account,
}

pub struct Status {
    pub text: String,
    pub is_error: bool,
}

/// What [`Session::next_event`] changed, so frontends refresh only that.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Changes {
    /// The playlist list was replaced (sync).
    pub library: bool,
    /// Tracks of this playlist changed (like, add to playlist).
    pub playlist: Option<String>,
    /// This video was liked or unliked.
    pub liked: Option<Arc<str>>,
    /// New online search results are in [`Session::search`].
    pub search: bool,
    /// OAuth client or sign-in state changed.
    pub account: bool,
}

/// Snapshot of what the UIs show; see [`Session::view`].
#[derive(Debug, Clone)]
pub struct SessionView {
    pub now: Option<Track>,
    pub loading: bool,
    pub state: PlayState,
    pub position: Duration,
    pub duration: Option<Duration>,
    pub volume: f32,
    pub shuffle: bool,
    pub repeat: Repeat,
    /// Status line text and whether it's an error.
    pub status: Option<(String, bool)>,
    pub syncing: bool,
    pub searching: bool,
    pub signed_in: bool,
    pub signing_in: bool,
    /// Configured OAuth client ID ("" when none).
    pub client_id: String,
    pub memory: String,
}

/// The last online search.
#[derive(Debug, Clone)]
pub struct SearchResults {
    pub query: String,
    pub tracks: Arc<[Track]>,
}

enum Background {
    Synced(Result<SyncReport>),
    Opened {
        generation: u64,
        result: Result<Box<Opened>>,
    },
    Rated {
        track: Track,
        liked: bool,
        result: Result<()>,
    },
    Added {
        playlist: Playlist,
        track: Track,
        result: Result<()>,
    },
    Searched {
        generation: u64,
        query: String,
        result: Result<Vec<Track>>,
    },
    SignedIn {
        generation: u64,
        result: Result<()>,
    },
    SignedOut(Result<()>),
}

pub struct Session {
    deps: Deps,
    player: Box<dyn Playback>,
    media: Option<MediaControls>,
    pub queue: Queue,
    /// Set while the current queue entry is being resolved/buffered.
    pub loading: bool,
    pub syncing: bool,
    /// Set while an online search runs.
    pub searching: bool,
    pub search: Option<SearchResults>,
    search_generation: u64,
    /// Waiting for the user to finish signing in in the browser.
    pub signing_in: bool,
    login_generation: u64,
    pub status: Option<Status>,
    pub player_status: PlayerStatus,
    /// "RAM 15 MB (+ yt-dlp …)", refreshed every [`MEMORY_SAMPLE`].
    pub memory: String,
    memory_at: Option<Instant>,
    memory_usage: sysmem::Usage,
    generation: u64,
    failures: u32,
    tx: UnboundedSender<Background>,
    rx: UnboundedReceiver<Background>,
    player_rx: UnboundedReceiver<PlayerEvent>,
    media_rx: UnboundedReceiver<MediaAction>,
}

impl Session {
    /// Starts the audio thread and media controls; restores saved modes.
    pub fn new(deps: Deps) -> Result<Self> {
        let (player_tx, player_rx) = mpsc::unbounded_channel();
        let (media_tx, media_rx) = mpsc::unbounded_channel();
        let volume = saved_volume(&deps);
        let player = PlayerHandle::spawn(volume, deps.audio_device.clone(), player_tx)?;
        let media = if deps.media_controls {
            MediaControls::new(media_tx)
        } else {
            None
        };
        Ok(Self::assemble(
            deps,
            Box::new(player),
            player_rx,
            media,
            media_rx,
        ))
    }

    /// A Session over another [`Playback`] adapter (tests), without OS
    /// media controls. `events` is where that adapter reports.
    pub fn with_playback(
        deps: Deps,
        player: Box<dyn Playback>,
        events: UnboundedReceiver<PlayerEvent>,
    ) -> Self {
        player.set_volume(saved_volume(&deps));
        let (_media_tx, media_rx) = mpsc::unbounded_channel();
        Self::assemble(deps, player, events, None, media_rx)
    }

    fn assemble(
        deps: Deps,
        player: Box<dyn Playback>,
        player_rx: UnboundedReceiver<PlayerEvent>,
        media: Option<MediaControls>,
        media_rx: UnboundedReceiver<MediaAction>,
    ) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        let meta = |key| deps.library.get_meta(key).ok().flatten();
        let mut queue = Queue::default();
        queue.shuffle = meta("shuffle").as_deref() == Some("1");
        queue.repeat = Repeat::parse(meta("repeat").as_deref().unwrap_or_default());
        Self {
            player_status: player.status(),
            deps,
            player,
            media,
            queue,
            loading: false,
            syncing: false,
            searching: false,
            search: None,
            search_generation: 0,
            signing_in: false,
            login_generation: 0,
            status: None,
            memory: String::new(),
            memory_at: None,
            memory_usage: sysmem::Usage::default(),
            generation: 0,
            failures: 0,
            tx,
            rx,
            player_rx,
            media_rx,
        }
    }

    /// Everything the UIs show about playback and background work, in one
    /// read (cheap: `Arc` strings, short texts).
    pub fn view(&self) -> SessionView {
        let p = &self.player_status;
        SessionView {
            now: self.queue.current().cloned(),
            loading: self.loading,
            state: p.state,
            position: p.position,
            duration: p.duration,
            volume: p.volume,
            shuffle: self.queue.shuffle,
            repeat: self.queue.repeat,
            status: self.status.as_ref().map(|s| (s.text.clone(), s.is_error)),
            syncing: self.syncing,
            searching: self.searching,
            signed_in: self.signed_in(),
            signing_in: self.signing_in,
            client_id: self.client_id().unwrap_or_default().to_owned(),
            memory: self.memory.clone(),
        }
    }

    /// Kicks off the first sync, or explains why there's nothing to show.
    pub fn startup(&mut self, library_empty: bool) {
        match (self.signed_in(), library_empty) {
            (true, true) => self.start_sync(),
            (false, true) => self.set_error(NOT_SIGNED_IN),
            (false, false) => self.set_info("Offline library (not signed in) — sign in to sync"),
            (true, false) => {}
        }
    }

    /// Persists volume and modes (the shown playlist is [`LibraryView::save`]).
    ///
    /// [`LibraryView::save`]: crate::library_view::LibraryView::save
    pub fn save_state(&self) {
        let repeat = self.queue.repeat.as_str();
        let volume = format!("{:.2}", self.player.status().volume);
        let shuffle = if self.queue.shuffle { "1" } else { "0" };
        let pairs = [
            ("volume", volume.as_str()),
            ("shuffle", shuffle),
            ("repeat", repeat),
        ];
        for (key, value) in pairs {
            if let Err(err) = self.deps.library.set_meta(key, value) {
                tracing::warn!(%err, key, "saving state");
            }
        }
    }

    pub fn shutdown(&mut self) {
        self.player.stop();
    }

    // --- events ---------------------------------------------------------------

    /// Re-reads player status (cheap) and, every few seconds, memory use;
    /// true when anything shown changed.
    pub fn refresh_status(&mut self) -> bool {
        let status = self.player.status();
        if let Some(media) = &mut self.media {
            media.set_state(status.state, status.position, false);
        }
        // Redraws are full-window repaints in the GUI (softbuffer hands out
        // a fresh buffer each frame on macOS), so only report what's visible:
        // whole seconds, volume percent.
        let mut changed = visible(&status) != visible(&self.player_status);
        self.player_status = status;

        if self.memory_at.is_none_or(|t| t.elapsed() >= MEMORY_SAMPLE) {
            self.memory_at = Some(Instant::now());
            let usage = sysmem::sample();
            // Allocator noise moves the footprint by ~1 MB every sample.
            if usage.differs_by(&self.memory_usage, MEMORY_STEP) {
                self.memory_usage = usage;
                self.memory = usage.label();
                changed = true;
            }
        }
        changed
    }

    /// Waits for the next background result, player or media-key event and
    /// applies it. Cancel-safe: may be raced in `select!`.
    pub async fn next_event(&mut self) -> Changes {
        tokio::select! {
            Some(event) = self.rx.recv() => self.on_background(event),
            Some(event) = self.player_rx.recv() => {
                self.on_player_event(event);
                Changes::default()
            }
            Some(action) = self.media_rx.recv() => {
                self.on_media(action);
                Changes::default()
            }
        }
    }

    fn on_background(&mut self, event: Background) -> Changes {
        let mut changes = Changes::default();
        match event {
            Background::Synced(result) => {
                self.syncing = false;
                match result {
                    Ok(r) => {
                        changes.library = true;
                        self.set_info(r.summary());
                    }
                    Err(err) => self.set_error(format!("Sync failed: {err:#}")),
                }
            }
            Background::Opened { generation, result } if generation == self.generation => {
                self.loading = false;
                match result {
                    Ok(opened) => {
                        let Opened { body, duration, .. } = *opened;
                        self.failures = 0;
                        self.player.load(body, duration, generation);
                        if let (Some(media), Some(track)) = (&mut self.media, self.queue.current())
                        {
                            media.set_track(track, duration);
                        }
                        self.prefetch_next();
                    }
                    Err(err) => self.track_failed(format!("{err:#}")),
                }
            }
            Background::Opened { .. } => {} // superseded by a newer selection
            Background::Rated {
                track,
                liked,
                result,
            } => match result {
                Ok(()) => {
                    let lib = &self.deps.library;
                    let stored = if liked {
                        lib.add_track(LIKED_PLAYLIST_ID, &track, true)
                    } else {
                        lib.remove_track(LIKED_PLAYLIST_ID, &track.video_id)
                    };
                    if let Err(err) = stored {
                        tracing::warn!(%err, "updating local likes");
                    }
                    changes.playlist = Some(LIKED_PLAYLIST_ID.into());
                    changes.liked = Some(track.video_id.clone());
                    let verb = if liked { "♥ Liked" } else { "Removed like:" };
                    self.set_info(format!("{verb} {}", track.title));
                }
                Err(err) => self.set_error(format!("{err:#}")),
            },
            Background::Added {
                playlist,
                track,
                result,
            } => match result {
                Ok(()) => {
                    if let Err(err) = self.deps.library.add_track(&playlist.id, &track, false) {
                        tracing::warn!(%err, "updating local playlist");
                    }
                    self.set_info(format!("Added {} to {}", track.title, playlist.title));
                    changes.playlist = Some(playlist.id);
                }
                Err(err) => self.set_error(format!("{err:#}")),
            },
            Background::Searched {
                generation,
                query,
                result,
            } if generation == self.search_generation => {
                self.searching = false;
                match result {
                    Ok(tracks) => {
                        self.set_info(format!("{} results for “{query}”", tracks.len()));
                        self.search = Some(SearchResults {
                            query,
                            tracks: tracks.into(),
                        });
                        changes.search = true;
                    }
                    Err(err) => self.set_error(format!("Search failed: {err:#}")),
                }
            }
            Background::Searched { .. } => {} // a newer search replaced it
            Background::SignedIn { generation, result } if generation == self.login_generation => {
                self.signing_in = false;
                changes.account = true;
                match result {
                    Ok(()) => {
                        self.deps.account.connect();
                        self.set_info("Signed in");
                        self.start_sync();
                    }
                    Err(err) => self.set_error(format!("Sign-in failed: {err:#}")),
                }
            }
            Background::SignedIn { .. } => {} // a newer attempt replaced it
            Background::SignedOut(result) => {
                changes.account = true;
                match result {
                    Ok(()) => self.set_info("Signed out — the library stays available offline"),
                    Err(err) => self.set_error(format!("Sign-out failed: {err:#}")),
                }
            }
        }
        changes
    }

    fn on_player_event(&mut self, event: PlayerEvent) {
        match event {
            PlayerEvent::Ended { generation } if generation == self.generation => {
                if self.queue.repeat == Repeat::One || self.queue.advance().is_some() {
                    self.play_current();
                } else if let Some(media) = &mut self.media {
                    media.set_state(PlayState::Idle, Duration::ZERO, true);
                }
            }
            PlayerEvent::Error {
                generation,
                message,
            } if generation == self.generation => self.track_failed(message),
            _ => {}
        }
    }

    fn on_media(&mut self, action: MediaAction) {
        match action {
            MediaAction::Toggle => self.toggle_pause(),
            MediaAction::Play => self.player.set_paused(false),
            MediaAction::Pause => self.player.set_paused(true),
            MediaAction::Next => self.skip(1),
            MediaAction::Prev => self.skip(-1),
            MediaAction::SeekBy(secs) => self.seek_by(secs),
            MediaAction::SeekTo(pos) => self.seek_to(pos),
        }
    }

    // --- playback ---------------------------------------------------------------

    /// Starts playing `tracks[index]`, with `tracks` as the queue.
    pub fn play(&mut self, tracks: Arc<[Track]>, index: usize) {
        self.queue.set(tracks, index);
        self.failures = 0;
        self.play_current();
    }

    /// Turns shuffle on and starts from a random track of `tracks`.
    pub fn play_shuffled(&mut self, tracks: Arc<[Track]>) {
        if tracks.is_empty() {
            return;
        }
        self.queue.shuffle = true;
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos() as usize);
        let start = seed % tracks.len();
        self.play(tracks, start);
    }

    /// Jumps to the `n`-th upcoming track (0 = next), as listed by
    /// `queue.upcoming()`.
    pub fn skip_ahead(&mut self, n: usize) {
        let mut moved = false;
        for _ in 0..=n {
            if self.queue.advance().is_none() {
                break;
            }
            moved = true;
        }
        if moved {
            self.play_current();
        }
    }

    pub fn toggle_pause(&mut self) {
        self.player.toggle_pause();
    }

    /// `direction` > 0: next; else previous (restarts a track > 3 s in).
    pub fn skip(&mut self, direction: i32) {
        let moved = if direction > 0 {
            self.queue.advance().is_some()
        } else {
            if self.player_status.position > Duration::from_secs(3)
                && self.queue.current().is_some()
            {
                self.seek_to(Duration::ZERO);
                return;
            }
            self.queue.back().is_some()
        };
        if moved {
            self.play_current();
        }
    }

    pub fn seek_by(&mut self, secs: i64) {
        self.player.seek_by(secs);
        self.media_resync();
    }

    pub fn seek_to(&mut self, position: Duration) {
        self.player.seek_to(position);
        self.media_resync();
    }

    /// Seeks to a fraction (0..=1) of the current track.
    pub fn seek_ratio(&mut self, ratio: f64) {
        if let Some(d) = self.player_status.duration {
            self.seek_to(d.mul_f64(ratio.clamp(0.0, 1.0)));
        }
    }

    pub fn change_volume(&mut self, delta: f32) {
        self.player.change_volume(delta);
    }

    pub fn set_volume(&mut self, volume: f32) {
        self.player.set_volume(volume);
    }

    pub fn toggle_shuffle(&mut self) {
        self.queue.toggle_shuffle();
        let state = if self.queue.shuffle { "on" } else { "off" };
        self.set_info(format!("Shuffle {state}"));
        self.prefetch_next();
    }

    pub fn cycle_repeat(&mut self) {
        self.queue.repeat = self.queue.repeat.cycle();
        self.set_info(format!("Repeat {}", self.queue.repeat.as_str()));
    }

    pub fn play_next(&mut self, track: Track) {
        self.set_info(format!("Playing next: {}", track.title));
        self.queue.play_next(track);
        self.prefetch_next();
    }

    fn media_resync(&mut self) {
        if let Some(media) = &mut self.media {
            let s = self.player.status();
            media.set_state(s.state, s.position, true);
        }
    }

    fn play_current(&mut self) {
        let Some(track) = self.queue.current().cloned() else {
            return;
        };
        self.generation += 1;
        self.loading = true;
        self.player.stop();
        self.status = None;

        let generation = self.generation;
        let (source, tx) = (self.deps.source.clone(), self.tx.clone());
        tokio::spawn(async move {
            let result = source.open(&track).await.map(Box::new);
            let _ = tx.send(Background::Opened { generation, result });
        });
    }

    fn prefetch_next(&self) {
        if let Some(next) = self.queue.peek_next() {
            self.deps.source.prefetch(&next.video_id);
        }
    }

    /// Shows the error and skips ahead, unless several tracks failed in a row.
    fn track_failed(&mut self, message: String) {
        self.loading = false;
        self.failures += 1;
        let title = self
            .queue
            .current()
            .map(|t| t.title.to_string())
            .unwrap_or_default();
        if self.failures < MAX_CONSECUTIVE_FAILURES && self.queue.advance().is_some() {
            self.play_current();
            self.set_error(format!("{title}: {message} — skipped"));
        } else {
            self.set_error(format!("{title}: {message}"));
        }
    }

    // --- library changes -----------------------------------------------------------

    fn youtube(&mut self) -> Option<Arc<YouTubeClient>> {
        if !self.signed_in() {
            self.set_error(NOT_SIGNED_IN);
        }
        self.deps.account.youtube()
    }

    pub fn start_sync(&mut self) {
        let Some(youtube) = self.youtube() else {
            return;
        };
        if self.syncing {
            return;
        }
        self.syncing = true;
        self.set_info("Syncing library…");
        let (library, music_only, tx) = (
            self.deps.library.clone(),
            self.deps.liked_music_only,
            self.tx.clone(),
        );
        tokio::spawn(async move {
            let result = sync_library(&youtube, library, music_only).await;
            let _ = tx.send(Background::Synced(result));
        });
    }

    /// Searches YouTube for music: the Data API when logged in (100 quota
    /// units), else — or when that fails, e.g. quota exhausted — yt-dlp.
    pub fn search(&mut self, query: &str) {
        let query = query.trim().to_owned();
        if query.is_empty() {
            return;
        }
        self.search_generation += 1;
        self.searching = true;
        self.set_info(format!("Searching “{query}”…"));
        let (youtube, source, tx, generation) = (
            self.deps.account.youtube(),
            self.deps.source.clone(),
            self.tx.clone(),
            self.search_generation,
        );
        tokio::spawn(async move {
            let api = match &youtube {
                Some(yt) => match yt.search(&query, SEARCH_RESULTS).await {
                    Ok(tracks) => Some(tracks),
                    Err(err) => {
                        tracing::warn!(%err, "API search failed; using yt-dlp");
                        None
                    }
                },
                None => None,
            };
            let result = match api {
                Some(tracks) => Ok(tracks),
                None => source.search(&query, SEARCH_RESULTS).await,
            };
            let _ = tx.send(Background::Searched {
                generation,
                query,
                result,
            });
        });
    }

    // --- account ------------------------------------------------------------------

    pub fn signed_in(&self) -> bool {
        self.deps.account.signed_in()
    }

    /// The configured OAuth client ID (browser sign-in), if any.
    pub fn client_id(&self) -> Option<&str> {
        self.deps.account.client_id(Flow::Browser)
    }

    /// Saves an OAuth client ("Desktop app") into `config.toml`.
    pub fn set_client(&mut self, id: &str, secret: &str) {
        let result = self.deps.account.set_client(Flow::Browser, id, secret);
        self.client_saved(result);
    }

    /// Reads Google's downloaded client JSON and saves it.
    pub fn import_client(&mut self, path: &Path) {
        let result = self.deps.account.import_client(Flow::Browser, path);
        self.client_saved(result.map(|_| ()));
    }

    fn client_saved(&mut self, result: Result<()>) {
        match result {
            Ok(()) if self.signed_in() => self.set_info("OAuth client saved"),
            Ok(()) => self.set_info("OAuth client saved — now sign in"),
            Err(err) => self.set_error(format!("{err:#}")),
        }
    }

    /// Opens Google's consent page in the browser and waits (in the
    /// background) for the redirect; then syncs.
    pub fn sign_in(&mut self) {
        let signing_in = self.deps.account.sign_in(Flow::Browser, |prompt| {
            if let Prompt::OpenUrl(url) = prompt {
                tracing::info!(%url, "sign-in page");
            }
        });
        let signing_in = match signing_in {
            Ok(future) => future,
            Err(err) => {
                self.set_error(format!("{err:#}"));
                return;
            }
        };
        self.login_generation += 1;
        self.signing_in = true;
        self.set_info("Finish signing in in your browser…");
        let (tx, generation) = (self.tx.clone(), self.login_generation);
        tokio::spawn(async move {
            let result = signing_in.await;
            let _ = tx.send(Background::SignedIn { generation, result });
        });
    }

    /// Forgets the stored token; the library stays for offline use.
    pub fn sign_out(&mut self) {
        self.login_generation += 1;
        self.signing_in = false;
        let clearing = self.deps.account.sign_out();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let _ = tx.send(Background::SignedOut(clearing.await));
        });
    }

    pub fn is_liked(&self, video_id: &str) -> bool {
        self.deps
            .library
            .contains(LIKED_PLAYLIST_ID, video_id)
            .unwrap_or(false)
    }

    /// Likes `track`, or removes the like if it's already liked.
    pub fn toggle_like(&mut self, track: Track) {
        let Some(youtube) = self.youtube() else {
            return;
        };
        let liked = !self.is_liked(&track.video_id);
        self.set_info(if liked {
            "Liking…"
        } else {
            "Removing like…"
        });
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = youtube.rate(&track.video_id, liked).await;
            let _ = tx.send(Background::Rated {
                track,
                liked,
                result,
            });
        });
    }

    /// Whether like / add-to-playlist are possible (shows why not).
    pub fn can_edit(&mut self) -> bool {
        self.youtube().is_some()
    }

    pub fn add_to_playlist(&mut self, playlist: Playlist, track: Track) {
        let Some(youtube) = self.youtube() else {
            return;
        };
        self.set_info(format!("Adding to {}…", playlist.title));
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = youtube.add_to_playlist(&playlist.id, &track.video_id).await;
            let _ = tx.send(Background::Added {
                playlist,
                track,
                result,
            });
        });
    }

    pub fn set_info(&mut self, text: impl Into<String>) {
        self.status = Some(Status {
            text: text.into(),
            is_error: false,
        });
    }

    pub fn set_error(&mut self, text: impl Into<String>) {
        let text = text.into();
        tracing::warn!("{text}");
        self.status = Some(Status {
            text,
            is_error: true,
        });
    }
}

/// The parts of the player status the UIs actually show.
fn visible(s: &PlayerStatus) -> (PlayState, u64, Option<u64>, u32) {
    (
        s.state,
        s.position.as_secs(),
        s.duration.map(|d| d.as_secs()),
        (s.volume * 100.0).round() as u32,
    )
}

/// Volume saved by the last run, else the config's start volume.
fn saved_volume(deps: &Deps) -> f32 {
    deps.library
        .get_meta("volume")
        .ok()
        .flatten()
        .and_then(|v| v.parse().ok())
        .unwrap_or(deps.volume)
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use futures_util::future::BoxFuture;
    use tokio::sync::mpsc::UnboundedSender;

    use super::*;
    use crate::{
        api::token_store::{InMemory, Tokens},
        audio::{
            Extractor, JsPolicy,
            extractor::{self, AudioStream, ExtractorError},
            js_runtime::JsRuntime,
            stream::HttpStream,
        },
        config::settings::Settings,
    };

    /// Records calls; its status is set by the test.
    #[derive(Clone)]
    struct FakePlayer {
        calls: Arc<Mutex<Vec<String>>>,
        status: Arc<Mutex<PlayerStatus>>,
    }

    impl FakePlayer {
        fn new() -> Self {
            Self {
                calls: Arc::default(),
                status: Arc::new(Mutex::new(PlayerStatus {
                    state: PlayState::Idle,
                    position: Duration::ZERO,
                    duration: None,
                    volume: 1.0,
                })),
            }
        }
        fn record(&self, call: String) {
            self.calls.lock().unwrap().push(call);
        }
        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
        fn at(&self, position: Duration) {
            let mut s = self.status.lock().unwrap();
            s.state = PlayState::Playing;
            s.position = position;
        }
    }

    impl Playback for FakePlayer {
        fn load(&self, _: HttpStream, _: Option<Duration>, generation: u64) {
            self.record(format!("load {generation}"));
        }
        fn toggle_pause(&self) {
            self.record("toggle".into());
        }
        fn set_paused(&self, paused: bool) {
            self.record(format!("paused {paused}"));
        }
        fn stop(&self) {
            self.record("stop".into());
        }
        fn seek_by(&self, secs: i64) {
            self.record(format!("seek_by {secs}"));
        }
        fn seek_to(&self, position: Duration) {
            self.record(format!("seek_to {}", position.as_secs()));
        }
        fn set_volume(&self, volume: f32) {
            self.status.lock().unwrap().volume = volume;
            self.record(format!("volume {volume:.2}"));
        }
        fn status(&self) -> PlayerStatus {
            self.status.lock().unwrap().clone()
        }
    }

    /// Every track is unavailable, or every resolve hangs (track "loading").
    #[derive(Clone, Copy)]
    enum Resolves {
        Fail,
        Hang,
    }

    impl Extractor for Resolves {
        fn resolve<'a>(
            &'a self,
            video_id: &'a str,
            _: Option<&'a JsRuntime>,
        ) -> BoxFuture<'a, extractor::Result<AudioStream>> {
            match self {
                Self::Fail => Box::pin(async move {
                    Err(ExtractorError::Unavailable(format!("{video_id} is gone")))
                }),
                Self::Hang => Box::pin(std::future::pending()),
            }
        }
        fn search<'a>(&'a self, _: &'a str, _: u8) -> BoxFuture<'a, extractor::Result<Vec<Track>>> {
            Box::pin(async { Ok(Vec::new()) })
        }
        fn update(&self) -> BoxFuture<'_, extractor::Result<String>> {
            Box::pin(async { Err(ExtractorError::NotManaged) })
        }
        fn update_if_stale<'a>(
            &'a self,
            _: &'a reqwest::Client,
            _: Duration,
        ) -> BoxFuture<'a, extractor::Result<bool>> {
            Box::pin(async { Ok(false) })
        }
    }

    async fn session_with(
        resolves: Resolves,
        library: Arc<Library>,
    ) -> (Session, FakePlayer, UnboundedSender<PlayerEvent>) {
        let http = reqwest::Client::new();
        let config = std::env::temp_dir().join(format!(
            "ytm-session-test-{}-{:?}.toml",
            std::process::id(),
            std::thread::current().id()
        ));
        let settings = Settings::load(&config).unwrap();
        let account = Account::load(
            &settings,
            config,
            http.clone(),
            Tokens::new(InMemory::default()),
        )
        .await
        .unwrap();
        let deps = Deps {
            library,
            source: TrackSource::new(
                Arc::new(resolves),
                http.clone(),
                std::env::temp_dir(),
                JsPolicy::Never,
                None,
            ),
            http,
            account,
            liked_music_only: true,
            volume: 0.5,
            media_controls: false,
            audio_device: None,
        };
        let player = FakePlayer::new();
        let (events, rx) = mpsc::unbounded_channel();
        let session = Session::with_playback(deps, Box::new(player.clone()), rx);
        (session, player, events)
    }

    async fn session(resolves: Resolves) -> (Session, FakePlayer, UnboundedSender<PlayerEvent>) {
        session_with(resolves, Arc::new(Library::open_in_memory().unwrap())).await
    }

    fn tracks(prefix: &str, n: usize) -> Arc<[Track]> {
        (0..n)
            .map(|i| Track {
                video_id: format!("{prefix}{i:0>10}").into(),
                title: format!("{prefix}{i}").into(),
                artist: "a".into(),
                duration_secs: Some(100),
            })
            .collect()
    }

    fn current(s: &Session) -> String {
        s.queue
            .current()
            .map_or("-".into(), |t| t.title.to_string())
    }

    /// Applies background results until nothing arrives for a while.
    async fn settle(s: &mut Session) {
        while tokio::time::timeout(Duration::from_millis(300), s.next_event())
            .await
            .is_ok()
        {}
    }

    #[tokio::test]
    async fn stops_skipping_after_three_failures_in_a_row() {
        let (mut s, _, _) = session(Resolves::Fail).await;
        s.play(tracks("t", 5), 0);
        settle(&mut s).await;
        assert_eq!(current(&s), "t2", "skipped twice, then stopped");
        let status = s.status.as_ref().unwrap();
        assert!(
            status.is_error && status.text.contains("t2"),
            "{}",
            status.text
        );
        assert!(!s.loading);
    }

    #[tokio::test]
    async fn results_of_a_replaced_selection_are_ignored() {
        let (mut s, _, _) = session(Resolves::Fail).await;
        s.play(tracks("old", 5), 0);
        s.play(tracks("new", 5), 0);
        settle(&mut s).await;
        // Only the new list's failures count: three of them.
        assert_eq!(current(&s), "new2");
    }

    #[tokio::test]
    async fn previous_restarts_the_track_after_three_seconds() {
        let (mut s, player, _) = session(Resolves::Hang).await;
        s.play(tracks("t", 3), 1);
        player.at(Duration::from_secs(5));
        s.refresh_status();
        s.skip(-1);
        assert_eq!(current(&s), "t1");
        assert!(player.calls().contains(&"seek_to 0".to_owned()));

        player.at(Duration::from_secs(1));
        s.refresh_status();
        s.skip(-1);
        assert_eq!(current(&s), "t0");
    }

    #[tokio::test]
    async fn natural_end_advances_repeat_one_replays_stale_ends_ignored() {
        let (mut s, _, events) = session(Resolves::Hang).await;
        s.play(tracks("t", 3), 0);
        events.send(PlayerEvent::Ended { generation: 1 }).unwrap();
        s.next_event().await;
        assert_eq!(current(&s), "t1");

        s.queue.repeat = Repeat::One;
        events.send(PlayerEvent::Ended { generation: 2 }).unwrap();
        s.next_event().await;
        assert_eq!(current(&s), "t1", "repeat one replays");

        events.send(PlayerEvent::Ended { generation: 1 }).unwrap();
        s.next_event().await;
        assert_eq!(current(&s), "t1", "an old track's end changes nothing");
    }

    #[tokio::test]
    async fn restores_saved_volume_and_modes() {
        let library = Arc::new(Library::open_in_memory().unwrap());
        library.set_meta("volume", "0.30").unwrap();
        library.set_meta("repeat", "all").unwrap();
        library.set_meta("shuffle", "1").unwrap();
        let (s, player, _) = session_with(Resolves::Hang, library).await;
        assert!(player.calls().contains(&"volume 0.30".to_owned()));
        assert_eq!(s.queue.repeat, Repeat::All);
        assert!(s.queue.shuffle);
    }

    #[tokio::test]
    async fn view_shows_the_loading_track() {
        let (mut s, _, _) = session(Resolves::Hang).await;
        s.play(tracks("t", 2), 1);
        let view = s.view();
        assert!(view.loading);
        assert_eq!(view.now.unwrap().title.as_ref(), "t1");
        assert!(!view.signed_in);
    }
}

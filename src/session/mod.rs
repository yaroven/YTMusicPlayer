//! UI-independent player core shared by the TUI and the GUI: queue and
//! playback (with preloading for gapless / crossfaded transitions, autoplay
//! radio and a sleep timer), library edits, browsing the YouTube Music
//! catalog, the account and media keys. Frontends call the action methods
//! and await [`Session::next_event`] to learn what changed.
//!
//! - `mod.rs`: state, events, playback;
//! - `library.rs`: account, sync, likes, playlists, follows, downloads;
//! - `discover.rs`: search, pages, home, radio, related, lyrics.

mod discover;
mod library;

use std::{
    collections::HashSet,
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::Result;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

pub use self::discover::{Found, SearchResults};
use crate::{
    account::Account,
    api::{
        client::Rating,
        models::{Playlist, Track},
    },
    audio::{
        Opened, TrackSource,
        player::{PlayState, Playback, PlayerEvent, PlayerHandle, PlayerStatus},
        queue::{Queue, Repeat},
    },
    catalog::{Catalog, Item, Page, SearchKind, Shelf},
    config::settings::Settings,
    lyrics::Lyrics,
    media::{MediaAction, MediaControls},
    storage::{Download, Library},
    sync::SyncReport,
    sysmem,
};

pub const SEEK_STEP_SECS: i64 = 5;
pub const VOLUME_STEP: f32 = 0.05;
/// Stop auto-skipping after this many tracks in a row fail to load.
const MAX_CONSECUTIVE_FAILURES: u32 = 3;
/// How often memory use is re-measured for display.
const MEMORY_SAMPLE: Duration = Duration::from_secs(2);
const NOT_SIGNED_IN: &str = "Not signed in: “Sign in” in the window, or `ytm login`";
/// Smallest memory change worth a redraw.
const MEMORY_STEP: u64 = 2 << 20;
/// The next track is opened this long before the current one ends (plus
/// the crossfade), so it starts without a gap.
const PRELOAD_LEAD: Duration = Duration::from_secs(20);

pub struct Deps {
    pub library: Arc<Library>,
    pub source: TrackSource,
    pub catalog: Catalog,
    pub http: reqwest::Client,
    pub liked_music_only: bool,
    pub volume: f32,
    pub media_controls: bool,
    pub audio_device: Option<String>,
    /// OAuth client, sign-in and the YouTube API client.
    pub account: Account,
    /// Keep playing the last song's radio when the queue runs out.
    pub autoplay: bool,
    /// Overlap between tracks (zero: gapless).
    pub crossfade: Duration,
    /// Told about every track that starts (scrobblers, Discord).
    pub listeners: Vec<Box<dyn Listener>>,
    /// Bytes of downloads after which no more are started.
    pub download_limit: Option<u64>,
}

/// Something that follows playback (Last.fm, Discord). Called on the core
/// thread; slow work belongs in spawned tasks.
pub trait Listener: Send {
    /// A track started playing (`duration` when known).
    fn started(&mut self, track: &Track, duration: Option<Duration>);
    /// Called about twice a second while something is loaded.
    fn progress(&mut self, _track: &Track, _position: Duration, _playing: bool) {}
    /// Playback stopped (queue ended, sleep timer).
    fn stopped(&mut self) {}
}

pub struct Status {
    pub text: String,
    pub is_error: bool,
}

/// When the sleep timer pauses playback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sleep {
    At(Instant),
    EndOfTrack,
}

/// What [`Session::next_event`] changed, so frontends refresh only that.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Changes {
    /// The playlist list was replaced (sync, playlist created / deleted).
    pub library: bool,
    /// Tracks of this playlist changed (like, add, remove, rename).
    pub playlist: Option<String>,
    /// This video was liked, unliked or disliked.
    pub liked: Option<Arc<str>>,
    /// New online search results are in [`Session::search`].
    pub search: bool,
    /// A page finished loading: [`Session::page`].
    pub page: bool,
    /// The home feed arrived: [`Session::home`].
    pub home: bool,
    /// Lyrics or related shelves for the playing track changed.
    pub lyrics: bool,
    pub related: bool,
    /// Saved albums / followed artists changed.
    pub saved: bool,
    /// Downloads changed.
    pub downloads: bool,
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
    pub autoplay: bool,
    /// Minutes left on the sleep timer, or "end of track".
    pub sleep: Option<Sleep>,
}

enum Background {
    Synced(Result<SyncReport>),
    Opened {
        generation: u64,
        result: Result<Box<Opened>>,
    },
    Preloaded {
        generation: u64,
        result: Result<Box<Opened>>,
    },
    /// Radio of `seed`: queued after the current track (`extend`) or
    /// replacing the queue.
    Radio {
        seed: Track,
        extend: bool,
        result: Result<Vec<Track>>,
    },
    Rated {
        track: Track,
        rating: Rating,
        result: Result<()>,
    },
    Added {
        playlist: Playlist,
        track: Track,
        result: Result<String>,
    },
    Removed {
        playlist: Playlist,
        track: Track,
        result: Result<()>,
    },
    PlaylistCreated {
        title: String,
        then_add: Option<Track>,
        result: Result<String>,
    },
    PlaylistRenamed {
        id: String,
        title: String,
        result: Result<()>,
    },
    PlaylistDeleted {
        playlist: Playlist,
        result: Result<()>,
    },
    Followed {
        item: Item,
        follow: bool,
        result: Result<()>,
    },
    Downloaded {
        track: Track,
        result: Result<Download>,
    },
    DownloadRemoved {
        video_id: Arc<str>,
        result: Result<()>,
    },
    Searched {
        generation: u64,
        query: String,
        kind: SearchKind,
        result: Result<Found>,
    },
    PageLoaded {
        generation: u64,
        result: Result<Page>,
    },
    Home(Result<Vec<Shelf>>),
    Related {
        video_id: Arc<str>,
        result: Result<Vec<Shelf>>,
    },
    Lyrics {
        video_id: Arc<str>,
        result: Result<Option<Lyrics>>,
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
    /// The last opened album / artist / playlist page.
    pub page: Option<Arc<Page>>,
    pub page_loading: bool,
    page_generation: u64,
    /// The home feed (YouTube Music's shelves).
    pub home: Option<Arc<[Shelf]>>,
    /// Lyrics of the playing track (when asked for with `want_lyrics`).
    pub lyrics: Option<(Arc<str>, Option<Arc<Lyrics>>)>,
    want_lyrics: bool,
    /// Related shelves of the playing track (when asked for).
    pub related: Option<(Arc<str>, Arc<[Shelf]>)>,
    want_related: bool,
    /// Tracks being downloaded.
    pub downloading: HashSet<Arc<str>>,
    /// Waiting for the user to finish signing in in the browser.
    pub signing_in: bool,
    login_generation: u64,
    pub status: Option<Status>,
    pub player_status: PlayerStatus,
    /// "RAM 15 MB (+ yt-dlp …)", refreshed every [`MEMORY_SAMPLE`].
    pub memory: String,
    memory_at: Option<Instant>,
    memory_usage: sysmem::Usage,
    /// Load id of the playing track; preloads get their own ids.
    generation: u64,
    loads: u64,
    /// The preload being opened or handed to the player, for the current
    /// generation.
    preload: Option<u64>,
    preload_for: Option<u64>,
    /// A radio request to extend the queue is in flight.
    extending: bool,
    pub sleep: Option<Sleep>,
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
        player.set_crossfade(deps.crossfade);
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
            page: None,
            page_loading: false,
            page_generation: 0,
            home: None,
            lyrics: None,
            want_lyrics: false,
            related: None,
            want_related: false,
            downloading: HashSet::new(),
            signing_in: false,
            login_generation: 0,
            status: None,
            memory: String::new(),
            memory_at: None,
            memory_usage: sysmem::Usage::default(),
            generation: 0,
            loads: 0,
            preload: None,
            preload_for: None,
            extending: false,
            sleep: None,
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
            autoplay: self.deps.autoplay,
            sleep: self.sleep,
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
        for listener in &mut self.deps.listeners {
            listener.stopped();
        }
    }

    pub fn library(&self) -> &Arc<Library> {
        &self.deps.library
    }

    // --- events ---------------------------------------------------------------

    /// Re-reads player status (cheap) and, every few seconds, memory use;
    /// starts preloading the next track near the end; true when anything
    /// shown changed.
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

        if let Some(track) = self.queue.current() {
            let (pos, playing) = (
                self.player_status.position,
                self.player_status.state == PlayState::Playing,
            );
            for listener in &mut self.deps.listeners {
                listener.progress(track, pos, playing);
            }
        }
        if let Some(Sleep::At(at)) = self.sleep
            && Instant::now() >= at
        {
            self.sleep = None;
            self.player.set_paused(true);
            self.set_info("Sleep timer: paused");
            changed = true;
        }
        self.maybe_preload();

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
            Background::Opened { generation, result } if generation == self.generation => {
                self.loading = false;
                match result {
                    Ok(opened) => {
                        self.failures = 0;
                        let duration = opened.duration;
                        self.player.load(opened.into_load(generation));
                        self.track_started(duration);
                    }
                    Err(err) => self.track_failed(format!("{err:#}")),
                }
            }
            Background::Opened { .. } => {} // superseded by a newer selection
            Background::Preloaded { generation, result } if self.preload == Some(generation) => {
                match result {
                    Ok(opened) => self.player.preload(opened.into_load(generation)),
                    // Not fatal: the normal path opens it when its turn comes.
                    Err(err) => {
                        tracing::debug!(%err, "preload failed");
                        self.preload = None;
                    }
                }
            }
            Background::Preloaded { .. } => {}
            Background::Radio {
                seed,
                extend,
                result,
            } => self.radio_arrived(seed, extend, result),
            Background::Synced(_)
            | Background::Rated { .. }
            | Background::Added { .. }
            | Background::Removed { .. }
            | Background::PlaylistCreated { .. }
            | Background::PlaylistRenamed { .. }
            | Background::PlaylistDeleted { .. }
            | Background::Followed { .. }
            | Background::Downloaded { .. }
            | Background::DownloadRemoved { .. }
            | Background::SignedIn { .. }
            | Background::SignedOut(_) => self.library_event(event, &mut changes),
            Background::Searched { .. }
            | Background::PageLoaded { .. }
            | Background::Home(_)
            | Background::Related { .. }
            | Background::Lyrics { .. } => self.discover_event(event, &mut changes),
        }
        changes
    }

    fn on_player_event(&mut self, event: PlayerEvent) {
        match event {
            PlayerEvent::Ended { generation } if generation == self.generation => {
                self.drop_preload();
                if self.sleep == Some(Sleep::EndOfTrack) {
                    self.sleep = None;
                    self.set_info("Sleep timer: stopped after the track");
                    self.stopped();
                } else if self.queue.repeat == Repeat::One || self.queue.advance().is_some() {
                    self.play_current();
                } else if self.deps.autoplay
                    && let Some(seed) = self.queue.current().cloned()
                {
                    self.set_info(format!("Autoplay: radio of {}", seed.title));
                    self.fetch_radio(seed, true);
                } else {
                    self.stopped();
                }
            }
            PlayerEvent::Started { generation } if self.preload == Some(generation) => {
                // The preloaded next track took over without a gap.
                self.preload = None;
                self.preload_for = None;
                self.queue.advance();
                self.generation = generation;
                self.failures = 0;
                let duration = self.player.status().duration;
                self.track_started(duration);
            }
            PlayerEvent::Started { .. } => {
                // A preload we no longer wanted: get back in sync.
                self.play_current();
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

    fn stopped(&mut self) {
        if let Some(media) = &mut self.media {
            media.set_state(PlayState::Idle, Duration::ZERO, true);
        }
        for listener in &mut self.deps.listeners {
            listener.stopped();
        }
    }

    /// The current queue entry is now playing.
    fn track_started(&mut self, duration: Option<Duration>) {
        let Some(track) = self.queue.current().cloned() else {
            return;
        };
        if let Some(media) = &mut self.media {
            media.set_track(&track, duration);
        }
        if let Err(err) = self.deps.library.add_history(&track) {
            tracing::warn!(%err, "recording history");
        }
        for listener in &mut self.deps.listeners {
            listener.started(&track, duration);
        }
        self.preload_for = None;
        self.prefetch_next();
        self.now_changed();
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
        } else if direction > 0
            && self.deps.autoplay
            && let Some(seed) = self.queue.current().cloned()
        {
            self.fetch_radio(seed, true);
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
        self.queue_changed();
        let state = if self.queue.shuffle { "on" } else { "off" };
        self.set_info(format!("Shuffle {state}"));
    }

    pub fn cycle_repeat(&mut self) {
        self.queue.repeat = self.queue.repeat.cycle();
        self.queue_changed();
        self.set_info(format!("Repeat {}", self.queue.repeat.as_str()));
    }

    pub fn toggle_autoplay(&mut self) {
        self.deps.autoplay = !self.deps.autoplay;
        let state = if self.deps.autoplay { "on" } else { "off" };
        self.set_info(format!("Autoplay {state}"));
        self.store_setting("autoplay", &self.deps.autoplay.to_string());
    }

    /// Changes the overlap between tracks (0: gapless).
    pub fn set_crossfade(&mut self, overlap: Duration) {
        self.deps.crossfade = overlap;
        self.player.set_crossfade(overlap);
        self.queue_changed();
        self.store_setting("crossfade", &overlap.as_secs().to_string());
    }

    /// Loudness normalization on or off (from the next track).
    pub fn set_normalize(&mut self, on: bool) {
        self.deps.source.set_normalize(on);
        self.store_setting("normalize_volume", &on.to_string());
    }

    /// Saves a preference into `config.toml` (`value` is a TOML literal).
    pub fn store_setting(&mut self, key: &str, value: &str) {
        let path = self.deps.account.config_file().to_owned();
        if let Err(err) = Settings::store_value(&path, key, value) {
            self.set_error(format!("Saving {key}: {err:#}"));
        }
    }

    pub fn play_next(&mut self, track: Track) {
        self.set_info(format!("Playing next: {}", track.title));
        self.queue.play_next(track);
        self.queue_changed();
    }

    /// Adds tracks to the end of the queue.
    pub fn add_to_queue(&mut self, tracks: Vec<Track>) {
        let n = tracks.len();
        self.queue.append(tracks);
        self.queue_changed();
        self.set_info(format!("Added {n} to the queue"));
    }

    /// Removes the `n`-th upcoming track.
    pub fn remove_from_queue(&mut self, n: usize) {
        self.queue.remove_upcoming(n);
        self.queue_changed();
    }

    /// Moves the `from`-th upcoming track to position `to`.
    pub fn move_in_queue(&mut self, from: usize, to: usize) {
        self.queue.move_upcoming(from, to);
        self.queue_changed();
    }

    /// Drops everything queued after the playing track.
    pub fn clear_queue(&mut self) {
        self.queue.clear_upcoming();
        self.queue_changed();
        self.set_info("Queue cleared");
    }

    /// Pauses after `minutes`, or at the end of the track (`None`).
    pub fn set_sleep(&mut self, minutes: Option<u32>) {
        self.sleep = Some(match minutes {
            Some(m) => Sleep::At(Instant::now() + Duration::from_secs(u64::from(m) * 60)),
            None => Sleep::EndOfTrack,
        });
        self.set_info(match minutes {
            Some(m) => format!("Sleeping in {m} min"),
            None => "Stopping after this track".into(),
        });
        // No preload past the end of this track.
        self.queue_changed();
    }

    pub fn cancel_sleep(&mut self) {
        self.sleep = None;
        self.set_info("Sleep timer off");
    }

    fn media_resync(&mut self) {
        if let Some(media) = &mut self.media {
            let s = self.player.status();
            media.set_state(s.state, s.position, true);
        }
    }

    fn next_load(&mut self) -> u64 {
        self.loads += 1;
        self.loads
    }

    fn play_current(&mut self) {
        self.drop_preload();
        let Some(track) = self.queue.current().cloned() else {
            return;
        };
        self.generation = self.next_load();
        self.loading = true;
        self.player.stop();
        self.status = None;

        let generation = self.generation;
        let (source, tx) = (self.deps.source.clone(), self.tx.clone());
        tokio::spawn(async move {
            let result = source.open(&track).await.map(Box::new);
            let _ = tx.send(Background::Opened { generation, result });
        });
        self.now_changed();
    }

    /// What follows the current track changed: the preload (if any) is
    /// for the wrong track now.
    fn queue_changed(&mut self) {
        self.drop_preload();
        self.prefetch_next();
    }

    fn drop_preload(&mut self) {
        if self.preload.take().is_some() {
            self.player.cancel_preload();
        }
        self.preload_for = None;
    }

    /// Near the end of the track: open the next one so the player can
    /// start it without a gap (or crossfade into it). With autoplay and
    /// nothing queued, fetch the radio first.
    fn maybe_preload(&mut self) {
        let s = &self.player_status;
        let Some(duration) = s.duration else { return };
        if s.state != PlayState::Playing
            || self.loading
            || self.preload_for == Some(self.generation)
            || self.queue.repeat == Repeat::One
            || self.sleep == Some(Sleep::EndOfTrack)
            || duration.saturating_sub(s.position) > PRELOAD_LEAD + self.deps.crossfade
        {
            return;
        }
        let Some(next) = self.queue.peek_next().cloned() else {
            if self.deps.autoplay
                && !self.extending
                && let Some(seed) = self.queue.current().cloned()
            {
                self.fetch_radio(seed, true);
            }
            return;
        };
        self.preload_for = Some(self.generation);
        let generation = self.next_load();
        self.preload = Some(generation);
        let (source, tx) = (self.deps.source.clone(), self.tx.clone());
        tokio::spawn(async move {
            let result = source.open(&next).await.map(Box::new);
            let _ = tx.send(Background::Preloaded { generation, result });
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
mod tests;

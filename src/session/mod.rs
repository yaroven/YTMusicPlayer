//! UI-independent player core shared by the TUI and the GUI: queue and
//! playback (with preloading for gapless / crossfaded transitions, autoplay
//! radio and a sleep timer), library edits, browsing the YouTube Music
//! catalog, the account and media keys. Frontends call the action methods
//! and await [`Session::next_event`] to learn what changed.
//!
//! - `mod.rs`: state, events, playback;
//! - `library.rs`: account, sync, likes, playlists, follows, downloads;
//! - `discover.rs`: search, pages, home, radio, related, lyrics.

mod cast;
mod discover;
mod library;
mod updates;
mod youtube;

use crate::youtube_session::YouTubeSession;
use std::{
    collections::{HashSet, VecDeque},
    sync::{Arc, atomic::AtomicBool},
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
    i18n::Lang,
    lyrics::Lyrics,
    media::{MediaAction, MediaControls},
    net,
    storage::{Download, Library},
    sync::SyncReport,
    sysmem, tr, trf, video,
};

pub const SEEK_STEP_SECS: i64 = 5;
pub const VOLUME_STEP: f32 = 0.05;
/// Stop auto-skipping after this many tracks in a row fail to load.
const MAX_CONSECUTIVE_FAILURES: u32 = 3;
/// How often memory use is re-measured for display.
const MEMORY_SAMPLE: Duration = Duration::from_secs(2);
/// Shown when something needs the account and there is none.
fn not_signed_in(lang: Lang) -> &'static str {
    tr!(
        lang,
        "Not signed in: Sign in → Sign in with Google (or `ytm sign-in`)",
        "Вхід не виконано: «Увійти» → «Увійти через Google» (або `ytm sign-in`)"
    )
}
/// Smallest memory change worth a redraw.
const MEMORY_STEP: u64 = 2 << 20;
/// The next track is opened this long before the current one ends (plus
/// the crossfade), so it starts without a gap.
const PRELOAD_LEAD: Duration = Duration::from_secs(20);
/// Pages kept for opening again at once.
const PAGE_CACHE: usize = 12;
/// A track that stops this long before its end ran dry (or failed).
const EARLY_END: Duration = Duration::from_secs(8);
/// Offline, YouTube Music is asked this often whether it answers again.
const PROBE_EVERY: Duration = Duration::from_secs(4);
/// The same track waits for the connection at most this many times in a
/// row (a connection too slow to ever load it shouldn't loop forever).
const MAX_NETWORK_RETRIES: u32 = 5;

pub struct Deps {
    pub library: Arc<Library>,
    pub source: TrackSource,
    pub catalog: Catalog,
    pub http: reqwest::Client,
    pub liked_music_only: bool,
    /// "auto", "en" or "uk" (`config.toml`).
    pub language: String,
    pub volume: f32,
    pub media_controls: bool,
    pub audio_device: Option<String>,
    /// OAuth client, sign-in and the YouTube API client.
    pub account: Account,
    /// The youtube.com sign-in ("Sign in with Google"): age-restricted
    /// songs, the account's YouTube Music.
    pub youtube: YouTubeSession,
    /// The browser (or cookies.txt) for age-restricted songs when not
    /// signed in to YouTube.
    pub cookies_from_browser: String,
    /// Keep playing the last song's radio when the queue runs out.
    pub autoplay: bool,
    /// Overlap between tracks (zero: gapless).
    pub crossfade: Duration,
    /// Told about every track that starts (scrobblers, Discord).
    pub listeners: Vec<Box<dyn Listener>>,
    /// Bytes of downloads after which no more are started.
    pub download_limit: Option<u64>,
    /// Look for a newer release at startup.
    pub check_updates: bool,
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
    /// Explore (charts, new releases, moods) arrived: [`Session::explore`].
    pub explore: bool,
    /// New search suggestions: [`Session::suggestions`].
    pub suggestions: bool,
    /// Lyrics or related shelves for the playing track changed.
    pub lyrics: bool,
    pub related: bool,
    /// Saved albums / followed artists changed.
    pub saved: bool,
    /// Downloads changed.
    pub downloads: bool,
    /// The history was edited (entries removed, cleared).
    pub history: bool,
    /// OAuth client or sign-in state changed.
    pub account: bool,
    /// Cast devices or the cast target changed.
    pub cast: bool,
    /// An update was found or installed.
    pub update: bool,
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
    /// The language the UI should use.
    pub lang: Lang,
    /// The clip of the playing song is open in the video window.
    pub watching: bool,
    /// How many songs are still being downloaded.
    pub downloading: usize,
    pub signed_in: bool,
    pub signing_in: bool,
    /// Signed in to YouTube ("Sign in with Google").
    pub youtube_signed_in: bool,
    pub youtube_signing_in: bool,
    /// Configured OAuth client ID ("" when none).
    pub client_id: String,
    pub memory: String,
    pub autoplay: bool,
    /// Minutes left on the sleep timer, or "end of track".
    pub sleep: Option<Sleep>,
    /// The Cast device playing, if any.
    pub casting: Option<String>,
    /// A newer version, when one was found.
    pub update: Option<String>,
    pub updating: bool,
    /// The last request failed for want of a connection (until one works).
    pub offline: bool,
}

enum Background {
    /// YouTube Music answers again (after `offline`), or still doesn't.
    Online(bool),
    /// A prefetched page (a card was pressed) came in.
    Prefetched {
        id: String,
        result: Result<Page>,
    },
    /// The track stopped well before its end: if the connection is gone,
    /// it continues at `at` when it's back.
    EndedEarly {
        generation: u64,
        at: Duration,
        online: bool,
    },
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
    /// The video window closed; the song continues where it stopped.
    VideoClosed {
        video_id: Arc<str>,
        result: Result<Duration>,
    },
    /// Downloads were cancelled; `stopped` were still queued or running.
    DownloadsStopped {
        stopped: usize,
    },
    /// Every downloaded track was deleted (or it failed).
    DownloadsCleared(Result<usize>),
    /// The queue was saved as a new playlist.
    QueueSaved {
        title: String,
        tracks: Vec<Track>,
        result: Result<String>,
    },
    /// A song moved inside one of the user's playlists (the local copy
    /// moved already; on an error it moves back).
    Moved {
        playlist_id: String,
        from: usize,
        to: usize,
        result: Result<()>,
    },
    /// History entries were removed.
    HistoryEdited(Result<()>),
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
    Explore(Result<Vec<Shelf>>),
    Suggestions {
        query: String,
        result: Result<Vec<String>>,
    },
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
    YouTubeSignedIn(Result<()>),
    YouTubeSignedOut(Result<()>),
    CastDevices(Result<Vec<crate::cast::Device>>),
    UpdateChecked {
        manual: bool,
        result: Result<Option<crate::update::Update>>,
    },
    UpdateInstalled(Result<crate::update::Installed>),
    CastConnected {
        name: String,
        result: Result<(crate::cast::CastPlayer, UnboundedReceiver<PlayerEvent>)>,
    },
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
    /// Explore shelves (charts, new releases, moods), once asked for.
    pub explore: Option<Arc<[Shelf]>>,
    /// What YouTube Music suggests for the query being typed.
    pub suggestions: Arc<[String]>,
    /// The query those suggestions are for (a late answer is dropped).
    suggesting: String,
    /// Lyrics of the playing track (when asked for with `want_lyrics`).
    pub lyrics: Option<(Arc<str>, Option<Arc<Lyrics>>)>,
    want_lyrics: bool,
    /// Related shelves of the playing track (when asked for).
    pub related: Option<(Arc<str>, Arc<[Shelf]>)>,
    want_related: bool,
    /// The language the player speaks ([`crate::i18n`]).
    pub lang: Lang,
    /// The clip is open in the video window.
    pub watching: bool,
    /// Tracks being downloaded.
    pub downloading: HashSet<Arc<str>>,
    /// The task working through them, so it can be stopped.
    download_task: Option<tokio::task::JoinHandle<()>>,
    /// Waiting for the user to finish signing in in the browser.
    pub signing_in: bool,
    login_generation: u64,
    /// The YouTube sign-in window (or browser import) is running.
    youtube_signing_in: bool,
    cookies_browser: String,
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
    /// Casting: the device's name, and the local player put aside.
    pub casting: Option<String>,
    local: Option<(Box<dyn Playback>, UnboundedReceiver<PlayerEvent>)>,
    cast_alive: Option<Arc<AtomicBool>>,
    /// Cast devices found by the last scan.
    pub cast_devices: Arc<[crate::cast::Device]>,
    pub cast_scanning: bool,
    /// Where to continue the track that is being opened (after a switch
    /// between local and cast output, or after the connection came back).
    resume_at: Option<Duration>,
    /// A request failed for want of a connection; a probe runs until
    /// YouTube Music answers again (`probing`).
    offline: bool,
    probing: bool,
    /// Playback stopped for want of a connection: it starts again (at
    /// `resume_at`) when the connection is back, up to a few times.
    waiting_for_network: bool,
    network_retries: u32,
    /// What failed for want of a connection, loaded again when it's back.
    retry_home: bool,
    retry_explore: bool,
    retry_page: Option<String>,
    /// The page being opened (by browse id), to open again after a failure.
    page_request: Option<String>,
    /// The last pages opened or prefetched, by browse id (newest last):
    /// opening one again is instant.
    page_cache: VecDeque<(String, Arc<Page>)>,
    /// Prefetches in flight (a card pressed, not yet released).
    prefetching: HashSet<String>,
    /// `open_page` found its page being prefetched: that result is the
    /// answer for this page generation.
    awaiting_prefetch: Option<(String, u64)>,
    /// A newer release (from the last check).
    pub update: Option<crate::update::Update>,
    checking_update: bool,
    updating: bool,
    /// What the last install did (the frontend restarts / quits).
    pub update_done: Option<crate::update::Installed>,
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
        let cookies_browser = deps.cookies_from_browser.clone();
        let lang = Lang::from_code(&deps.language);
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
            explore: None,
            suggestions: Arc::from([]),
            suggesting: String::new(),
            lyrics: None,
            want_lyrics: false,
            related: None,
            want_related: false,
            lang,
            watching: false,
            downloading: HashSet::new(),
            download_task: None,
            signing_in: false,
            login_generation: 0,
            youtube_signing_in: false,
            cookies_browser,
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
            casting: None,
            local: None,
            cast_alive: None,
            cast_devices: Arc::from([]),
            cast_scanning: false,
            resume_at: None,
            offline: false,
            probing: false,
            waiting_for_network: false,
            network_retries: 0,
            retry_home: false,
            retry_explore: false,
            retry_page: None,
            page_request: None,
            page_cache: VecDeque::new(),
            prefetching: HashSet::new(),
            awaiting_prefetch: None,
            update: None,
            checking_update: false,
            updating: false,
            update_done: None,
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
            lang: self.lang,
            watching: self.watching,
            downloading: self.downloading.len(),
            signed_in: self.signed_in(),
            signing_in: self.signing_in,
            youtube_signed_in: self.deps.youtube.signed_in(),
            youtube_signing_in: self.youtube_signing_in,
            client_id: self.client_id().unwrap_or_default().to_owned(),
            memory: self.memory.clone(),
            autoplay: self.deps.autoplay,
            sleep: self.sleep,
            casting: self.casting.clone(),
            update: self.update.as_ref().map(|u| u.version.clone()),
            updating: self.updating,
            offline: self.offline,
        }
    }

    /// Kicks off the first sync, or explains why there's nothing to show.
    pub fn startup(&mut self, library_empty: bool) {
        if self.deps.check_updates {
            self.check_for_update(false);
        }
        // Either sign-in can sync: the session through InnerTube, the OAuth
        // client through the Data API.
        let can_sync = self.signed_in() || self.deps.youtube.signed_in();
        match (can_sync, library_empty) {
            (true, true) => self.start_sync(),
            (false, true) => self.set_error(not_signed_in(self.lang)),
            (false, false) => self.set_info(tr!(
                self.lang,
                "Offline library (not signed in) — sign in to sync",
                "Офлайн-бібліотека (вхід не виконано) — увійди, щоб синхронізувати"
            )),
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
        self.check_cast();
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
            self.set_info(tr!(self.lang, "Sleep timer: paused", "Таймер сну: пауза"));
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
                        let mut load = opened.into_load(generation);
                        load.track = self.queue.current().cloned();
                        self.player.load(load);
                        if let Some(position) = self.resume_at.take() {
                            self.player.seek_to(position);
                        }
                        self.track_started(duration);
                    }
                    Err(err) if net::is_network(&err) => {
                        let reason = net::describe(self.lang, &err);
                        self.wait_for_network(None, reason);
                    }
                    Err(err) => self.track_failed(format!("{err:#}")),
                }
            }
            Background::Opened { .. } => {} // superseded by a newer selection
            Background::Online(true) => {
                self.probing = false;
                if std::mem::take(&mut self.offline) {
                    self.set_info(tr!(self.lang, "Back online", "Інтернет знову є"));
                }
                if std::mem::take(&mut self.waiting_for_network) {
                    self.play_current();
                }
                if std::mem::take(&mut self.retry_home) {
                    self.load_home(true);
                }
                if std::mem::take(&mut self.retry_explore) {
                    self.load_explore(true);
                }
                if let Some(id) = self.retry_page.take() {
                    self.open_page(&id);
                }
            }
            Background::Online(false) => {
                self.set_error(net::offline_text(self.lang));
                self.went_offline();
            }
            Background::EndedEarly {
                generation,
                at,
                online,
            } if generation == self.generation => {
                if online {
                    self.track_ended();
                } else {
                    let reason = net::offline_text(self.lang).to_owned();
                    self.wait_for_network(Some(at), reason);
                }
            }
            Background::EndedEarly { .. } => {}
            Background::Preloaded { generation, result } if self.preload == Some(generation) => {
                match result {
                    Ok(opened) => {
                        let mut load = opened.into_load(generation);
                        load.track = self.queue.peek_next().cloned();
                        self.player.preload(load);
                    }
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
            | Background::VideoClosed { .. }
            | Background::DownloadsStopped { .. }
            | Background::DownloadsCleared(_)
            | Background::QueueSaved { .. }
            | Background::Moved { .. }
            | Background::HistoryEdited(_)
            | Background::SignedIn { .. }
            | Background::SignedOut(_) => self.library_event(event, &mut changes),
            Background::YouTubeSignedIn(_) | Background::YouTubeSignedOut(_) => {
                self.youtube_event(event, &mut changes)
            }
            Background::Searched { .. }
            | Background::PageLoaded { .. }
            | Background::Prefetched { .. }
            | Background::Home(_)
            | Background::Explore(_)
            | Background::Suggestions { .. }
            | Background::Related { .. }
            | Background::Lyrics { .. } => self.discover_event(event, &mut changes),
            Background::CastDevices(_) | Background::CastConnected { .. } => {
                self.cast_event(event, &mut changes)
            }
            Background::UpdateChecked { .. } | Background::UpdateInstalled(_) => {
                self.update_event(event, &mut changes)
            }
        }
        changes
    }

    fn on_player_event(&mut self, event: PlayerEvent) {
        match event {
            PlayerEvent::Ended { generation } if generation == self.generation => {
                // Well before its end: the stream may have run dry for want
                // of a connection. Ask before moving on.
                let s = &self.player_status;
                let early = self.casting.is_none()
                    && s.duration
                        .is_some_and(|d| d.saturating_sub(s.position) > EARLY_END);
                if early {
                    let (tx, at) = (self.tx.clone(), s.position);
                    tokio::spawn(async move {
                        let online = net::online().await;
                        let _ = tx.send(Background::EndedEarly {
                            generation,
                            at,
                            online,
                        });
                    });
                } else {
                    self.track_ended();
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
            } if generation == self.generation => {
                if net::problem_in_text(&message) == net::Problem::Other {
                    self.track_failed(message);
                } else {
                    let at = self.player_status.position;
                    let reason = net::offline_text(self.lang).to_owned();
                    self.wait_for_network(Some(at), reason);
                }
            }
            _ => {}
        }
    }

    /// The track played to its end: the next one, radio, or stop.
    fn track_ended(&mut self) {
        self.drop_preload();
        if self.sleep == Some(Sleep::EndOfTrack) {
            self.sleep = None;
            self.set_info(tr!(
                self.lang,
                "Sleep timer: stopped after the track",
                "Таймер сну: зупинено після пісні"
            ));
            self.stopped();
        } else if self.queue.repeat == Repeat::One || self.queue.advance().is_some() {
            self.play_current();
        } else if self.deps.autoplay
            && let Some(seed) = self.queue.current().cloned()
        {
            self.set_info(trf!(
                self.lang,
                "Autoplay: radio of {}",
                "Автоплей: радіо за {}",
                seed.title
            ));
            self.fetch_radio(seed, true);
        } else {
            self.stopped();
        }
    }

    /// Playback stopped for want of a connection: keep the track (and,
    /// mid-song, the place) and start again when the connection is back,
    /// instead of failing through the queue.
    fn wait_for_network(&mut self, at: Option<Duration>, reason: String) {
        self.loading = false;
        self.network_retries += 1;
        if self.network_retries > MAX_NETWORK_RETRIES {
            self.network_retries = 0;
            self.track_failed(reason);
            return;
        }
        if at.is_some() {
            self.resume_at = at;
        }
        self.waiting_for_network = true;
        self.went_offline();
        self.set_error(trf!(
            self.lang,
            "{reason} — playback continues when it's back",
            "{reason} — відтворення продовжиться, щойно він з'явиться"
        ));
    }

    /// A request failed for want of a connection: watch for it to return.
    fn went_offline(&mut self) {
        self.offline = true;
        if std::mem::replace(&mut self.probing, true) {
            return;
        }
        let tx = self.tx.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(PROBE_EVERY).await;
                if net::online().await {
                    let _ = tx.send(Background::Online(true));
                    break;
                }
            }
        });
    }

    /// `err` is a network failure: note it (and start watching). The
    /// status line then says so in plain words via [`net::describe`].
    pub(crate) fn network_failed(&mut self, err: &anyhow::Error) -> bool {
        let network = net::is_network(err);
        if network {
            self.went_offline();
        }
        network
    }

    /// A request failed for want of a connection, and none has worked since.
    pub fn is_offline(&self) -> bool {
        self.offline
    }

    /// "Retry": checks the connection now and, if it's there, loads again
    /// whatever failed for want of it.
    pub fn retry(&mut self) {
        let tx = self.tx.clone();
        self.set_info(tr!(
            self.lang,
            "Checking the connection…",
            "Перевіряю з'єднання…"
        ));
        tokio::spawn(async move {
            let _ = tx.send(Background::Online(net::online().await));
        });
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
        self.network_retries = 0;
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

    /// The window switched to the video: the audio waits there.
    pub fn pause_for_video(&mut self) {
        self.watching = true;
        self.player.set_paused(true);
        self.set_info(tr!(
            self.lang,
            "Watching the video — the audio waits here",
            "Дивишся відео — аудіо чекає тут"
        ));
    }

    /// Back from the video: the song carries on where the video stopped.
    pub fn resume_from_video(&mut self, at: Option<Duration>) {
        self.watching = false;
        if let Some(at) = at {
            self.seek_to(at);
        }
        self.player.set_paused(false);
        self.set_info(tr!(self.lang, "Back to the song", "Назад до пісні"));
    }

    /// Watches the playing song's clip: the audio stops, and when the video
    /// window closes the song carries on where the video stopped. Without a
    /// system web view (Linux) the clip opens in the browser instead.
    pub fn watch_video(&mut self) {
        let Some(track) = self.queue.current().cloned() else {
            self.set_error(tr!(self.lang, "Nothing is playing", "Нічого не грає"));
            return;
        };
        if self.watching {
            self.set_info(tr!(
                self.lang,
                "The video is already open",
                "Відео вже відкрите"
            ));
            return;
        }
        let at = self.player_status.position;
        self.player.set_paused(true);
        if !video::in_window() {
            let url = video::watch_url(&track.video_id, at);
            match open::that_detached(&url) {
                Ok(_) => self.set_info(trf!(
                    self.lang,
                    "Watching {} in your browser",
                    "Дивишся {} у браузері",
                    track.title
                )),
                Err(err) => self.set_error(trf!(
                    self.lang,
                    "Opening the video: {err}",
                    "Відкриття відео: {err}"
                )),
            }
            return;
        }
        self.watching = true;
        self.set_info(trf!(
            self.lang,
            "Watching {} — the audio continues when you close the window",
            "Дивишся {} — аудіо продовжиться, коли закриєш вікно",
            track.title
        ));
        let (tx, video_id) = (self.tx.clone(), track.video_id.clone());
        tokio::spawn(async move {
            let result = video::watch(&video_id, at).await;
            let _ = tx.send(Background::VideoClosed { video_id, result });
        });
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
        let state = on_off(self.lang, self.queue.shuffle);
        self.set_info(trf!(self.lang, "Shuffle {state}", "Перемішування {state}"));
    }

    pub fn cycle_repeat(&mut self) {
        self.queue.repeat = self.queue.repeat.cycle();
        self.queue_changed();
        let mode = repeat_label(self.lang, self.queue.repeat);
        self.set_info(trf!(self.lang, "Repeat {mode}", "Повтор {mode}"));
    }

    pub fn toggle_autoplay(&mut self) {
        self.deps.autoplay = !self.deps.autoplay;
        let state = on_off(self.lang, self.deps.autoplay);
        self.set_info(trf!(self.lang, "Autoplay {state}", "Автоплей {state}"));
        self.store_setting("autoplay", &self.deps.autoplay.to_string());
    }

    /// Changes the overlap between tracks (0: gapless).
    pub fn set_crossfade(&mut self, overlap: Duration) {
        self.deps.crossfade = overlap;
        self.player.set_crossfade(overlap);
        self.queue_changed();
        self.store_setting("crossfade", &overlap.as_secs().to_string());
    }

    /// Switches the language of everything the player says; kept in
    /// `config.toml`.
    pub fn set_language(&mut self, code: &str) {
        self.lang = Lang::from_code(code);
        self.store_setting("language", &format!("{code:?}"));
    }

    /// Loudness normalization on or off (from the next track).
    pub fn set_normalize(&mut self, on: bool) {
        self.deps.source.set_normalize(on);
        self.store_setting("normalize_volume", &on.to_string());
    }

    /// Lets age-restricted songs use `browser`'s YouTube login ("" = off).
    pub fn set_cookies_browser(&mut self, browser: &str) {
        browser.clone_into(&mut self.cookies_browser);
        // The YouTube sign-in, when there is one, stays in charge.
        if !self.deps.youtube.signed_in() {
            self.deps.source.set_cookies_browser(browser);
        }
        self.store_setting("cookies_from_browser", &format!("{browser:?}"));
    }

    /// Saves a preference into `config.toml` (`value` is a TOML literal).
    pub fn store_setting(&mut self, key: &str, value: &str) {
        let path = self.deps.account.config_file().to_owned();
        if let Err(err) = Settings::store_value(&path, key, value) {
            self.set_error(trf!(
                self.lang,
                "Saving {key}: {err:#}",
                "Збереження {key}: {err:#}"
            ));
        }
    }

    pub fn play_next(&mut self, track: Track) {
        self.set_info(trf!(
            self.lang,
            "Playing next: {}",
            "Наступною: {}",
            track.title
        ));
        self.queue.play_next(track);
        self.queue_changed();
    }

    /// Adds tracks to the end of the queue.
    pub fn add_to_queue(&mut self, tracks: Vec<Track>) {
        let n = tracks.len();
        self.queue.append(tracks);
        self.queue_changed();
        self.set_info(trf!(
            self.lang,
            "Added {n} to the queue",
            "Додано {n} у чергу"
        ));
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
        self.set_info(tr!(self.lang, "Queue cleared", "Чергу очищено"));
    }

    /// Pauses after `minutes`, or at the end of the track (`None`).
    pub fn set_sleep(&mut self, minutes: Option<u32>) {
        self.sleep = Some(match minutes {
            Some(m) => Sleep::At(Instant::now() + Duration::from_secs(u64::from(m) * 60)),
            None => Sleep::EndOfTrack,
        });
        self.set_info(match minutes {
            Some(m) => trf!(self.lang, "Sleeping in {m} min", "Сон за {m} хв"),
            None => tr!(
                self.lang,
                "Stopping after this track",
                "Зупинка після цієї пісні"
            )
            .into(),
        });
        // No preload past the end of this track.
        self.queue_changed();
    }

    pub fn cancel_sleep(&mut self) {
        self.sleep = None;
        self.set_info(tr!(self.lang, "Sleep timer off", "Таймер сну вимкнено"));
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
        self.waiting_for_network = false;
        self.drop_preload();
        let Some(track) = self.queue.current().cloned() else {
            return;
        };
        self.generation = self.next_load();
        self.loading = true;
        self.player.stop();
        self.status = None;

        let generation = self.generation;
        let (source, tx, remote) = (
            self.deps.source.clone(),
            self.tx.clone(),
            self.casting.is_some(),
        );
        tokio::spawn(async move {
            let result = source.open(&track, remote).await.map(Box::new);
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
        let (source, tx, remote) = (
            self.deps.source.clone(),
            self.tx.clone(),
            self.casting.is_some(),
        );
        tokio::spawn(async move {
            let result = source.open(&next, remote).await.map(Box::new);
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
            self.set_error(trf!(
                self.lang,
                "{title}: {message} — skipped",
                "{title}: {message} — пропущено"
            ));
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

/// "on" / "off" in the chosen language.
fn on_off(lang: Lang, on: bool) -> &'static str {
    match (lang, on) {
        (Lang::En, true) => "on",
        (Lang::En, false) => "off",
        (Lang::Uk, true) => "увімкнено",
        (Lang::Uk, false) => "вимкнено",
    }
}

/// "off" / "all" / "one" for the repeat mode.
fn repeat_label(lang: Lang, repeat: Repeat) -> &'static str {
    match (lang, repeat) {
        (Lang::En, _) => repeat.as_str(),
        (Lang::Uk, Repeat::Off) => "вимкнено",
        (Lang::Uk, Repeat::All) => "усе",
        (Lang::Uk, Repeat::One) => "одна",
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

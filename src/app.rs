//! Application root: state, the main `tokio::select!` loop (keys and mouse,
//! background results, player and media-key events) and the actions behind
//! each input. Redraws only when something changed.

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::Result;
use crossterm::{
    event::{
        DisableMouseCapture, EnableMouseCapture, Event, EventStream, KeyCode, KeyEvent,
        KeyEventKind, MouseButton, MouseEvent, MouseEventKind,
    },
    execute,
};
use futures_util::StreamExt;
use ratatui::{DefaultTerminal, layout::Rect, widgets::ListState};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

use crate::{
    api::{
        client::YouTubeClient,
        models::{LIKED_PLAYLIST_ID, Playlist, Track},
    },
    audio::{
        extractor::AudioStream,
        open_track,
        player::{PlayState, PlayerEvent, PlayerHandle, PlayerStatus},
        queue::{Queue, Repeat},
        resolver::StreamResolver,
        stream::HttpStream,
    },
    media::{MediaAction, MediaControls},
    storage::Library,
    sync::{SyncReport, sync_library},
    ui::{self, keymap::Action},
};

const SEEK_STEP_SECS: i64 = 5;
const VOLUME_STEP: f32 = 0.05;
/// Stop auto-skipping after this many tracks in a row fail to load.
const MAX_CONSECUTIVE_FAILURES: u32 = 3;
const DOUBLE_CLICK: Duration = Duration::from_millis(400);

pub struct Deps {
    pub library: Arc<Library>,
    pub resolver: StreamResolver,
    pub http: reqwest::Client,
    /// `None` when not logged in.
    pub youtube: Option<Arc<YouTubeClient>>,
    pub liked_music_only: bool,
    pub volume: f32,
    pub media_controls: bool,
    pub audio_device: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Playlists,
    Tracks,
}

/// What the right-hand pane shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TracksView {
    Playlist,
    Queue,
}

pub enum Mode {
    Normal,
    /// Typing a filter for the track list.
    Search,
    /// Choosing a playlist to add `track` to.
    AddTo {
        track: Track,
        state: ListState,
    },
    Help,
}

enum AppEvent {
    Synced(Result<SyncReport>),
    Opened {
        generation: u64,
        result: Result<Box<(HttpStream, AudioStream)>>,
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
}

pub struct Status {
    pub text: String,
    pub is_error: bool,
}

/// Screen regions from the last draw, for mouse hit-testing.
#[derive(Default, Clone, Copy)]
pub struct Areas {
    pub playlists: Rect,
    pub tracks: Rect,
    pub progress: Rect,
}

pub struct App {
    pub(crate) playlists: Vec<Playlist>,
    pub(crate) playlist_state: ListState,
    /// Tracks of the selected playlist, shared with the queue.
    pub(crate) tracks: Arc<[Track]>,
    /// Indices into `tracks` matching the filter (all when no filter).
    pub(crate) visible: Vec<u32>,
    pub(crate) filter: String,
    /// Selected row in `visible`, and the first row on screen.
    pub(crate) track_selected: Option<usize>,
    pub(crate) track_offset: usize,
    pub(crate) focus: Focus,
    pub(crate) view: TracksView,
    pub(crate) mode: Mode,
    pub(crate) queue: Queue,
    /// Set while the current queue entry is being resolved/buffered.
    pub(crate) loading: bool,
    pub(crate) status: Option<Status>,
    pub(crate) syncing: bool,
    pub(crate) logged_in: bool,
    pub(crate) player_status: PlayerStatus,
    pub(crate) areas: Areas,
    generation: u64,
    failures: u32,
    quit: bool,
    last_click: Option<(Instant, usize)>,
    player: PlayerHandle,
    media: Option<MediaControls>,
    deps: Deps,
    tx: UnboundedSender<AppEvent>,
}

pub async fn run(deps: Deps) -> Result<()> {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let (player_tx, mut player_rx) = mpsc::unbounded_channel();
    let (media_tx, mut media_rx) = mpsc::unbounded_channel();

    let volume = deps
        .library
        .get_meta("volume")
        .ok()
        .flatten()
        .and_then(|v| v.parse().ok())
        .unwrap_or(deps.volume);
    let player = PlayerHandle::spawn(volume, deps.audio_device.clone(), player_tx)?;
    let media = if deps.media_controls {
        MediaControls::new(media_tx)
    } else {
        None
    };

    let mut app = App {
        playlists: Vec::new(),
        playlist_state: ListState::default(),
        tracks: Arc::from([]),
        visible: Vec::new(),
        filter: String::new(),
        track_selected: None,
        track_offset: 0,
        focus: Focus::Playlists,
        view: TracksView::Playlist,
        mode: Mode::Normal,
        queue: Queue::default(),
        loading: false,
        status: None,
        syncing: false,
        logged_in: deps.youtube.is_some(),
        player_status: player.status(),
        areas: Areas::default(),
        generation: 0,
        failures: 0,
        quit: false,
        last_click: None,
        player,
        media,
        deps,
        tx,
    };
    app.startup();

    // Restores the terminal on panic too (installs a panic hook).
    let mut terminal = ratatui::init();
    let _ = execute!(std::io::stdout(), EnableMouseCapture);
    let result = app
        .event_loop(&mut terminal, &mut rx, &mut player_rx, &mut media_rx)
        .await;
    let _ = execute!(std::io::stdout(), DisableMouseCapture);
    ratatui::restore();
    app.save_state();
    result
}

impl App {
    fn startup(&mut self) {
        let lib = &self.deps.library;
        self.queue.shuffle = lib.get_meta("shuffle").ok().flatten().as_deref() == Some("1");
        self.queue.repeat = match lib.get_meta("repeat").ok().flatten().as_deref() {
            Some("all") => Repeat::All,
            Some("one") => Repeat::One,
            _ => Repeat::Off,
        };
        let last_playlist = lib.get_meta("playlist").ok().flatten();

        self.reload_playlists(last_playlist.as_deref());
        match (self.logged_in, self.playlists.is_empty()) {
            (true, true) => self.start_sync(),
            (false, true) => self.set_error("Not logged in: quit (q) and run `ytm login`"),
            (false, false) => {
                self.set_info("Offline library (not logged in) — `ytm login` to sync")
            }
            (true, false) => {}
        }
    }

    fn save_state(&self) {
        let repeat = match self.queue.repeat {
            Repeat::Off => "off",
            Repeat::All => "all",
            Repeat::One => "one",
        };
        let volume = format!("{:.2}", self.player.status().volume);
        let shuffle = if self.queue.shuffle { "1" } else { "0" };
        let mut pairs = vec![
            ("volume", volume.as_str()),
            ("shuffle", shuffle),
            ("repeat", repeat),
        ];
        if let Some(p) = self.selected_playlist() {
            pairs.push(("playlist", p.id.as_str()));
        }
        for (key, value) in pairs {
            if let Err(err) = self.deps.library.set_meta(key, value) {
                tracing::warn!(%err, key, "saving state");
            }
        }
    }

    async fn event_loop(
        &mut self,
        terminal: &mut DefaultTerminal,
        rx: &mut UnboundedReceiver<AppEvent>,
        player_rx: &mut UnboundedReceiver<PlayerEvent>,
        media_rx: &mut UnboundedReceiver<MediaAction>,
    ) -> Result<()> {
        let mut input = EventStream::new();
        // Progress bar cadence while playing; idle redraws only on input.
        let mut tick = tokio::time::interval(Duration::from_millis(500));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut dirty = true;

        while !self.quit {
            let status = self.player.status();
            if status != self.player_status {
                self.player_status = status;
                dirty = true;
            }
            if let Some(media) = &mut self.media {
                media.set_state(self.player_status.state, self.player_status.position, false);
            }
            if dirty {
                terminal.draw(|frame| ui::draw(frame, self))?;
                dirty = false;
            }

            tokio::select! {
                Some(event) = input.next() => {
                    match event? {
                        // Windows also reports key releases.
                        Event::Key(key) if key.kind != KeyEventKind::Release => self.on_key(key),
                        Event::Mouse(mouse) => self.on_mouse(mouse),
                        _ => {}
                    }
                    dirty = true;
                }
                Some(event) = rx.recv() => { self.on_app_event(event); dirty = true; }
                Some(event) = player_rx.recv() => { self.on_player_event(event); dirty = true; }
                Some(action) = media_rx.recv() => { self.on_media(action); dirty = true; }
                _ = tick.tick() => {}
            }
        }
        self.player.stop();
        Ok(())
    }

    // --- input -----------------------------------------------------------------

    fn on_key(&mut self, key: KeyEvent) {
        match &mut self.mode {
            Mode::Help => {
                self.mode = Mode::Normal;
                return;
            }
            Mode::Search => {
                match key.code {
                    KeyCode::Esc => {
                        self.filter.clear();
                        self.mode = Mode::Normal;
                    }
                    KeyCode::Enter => {
                        self.mode = Mode::Normal;
                        self.focus = Focus::Tracks;
                        return;
                    }
                    KeyCode::Backspace => {
                        self.filter.pop();
                    }
                    KeyCode::Char(c) => self.filter.push(c),
                    _ => return,
                }
                self.apply_filter();
                return;
            }
            Mode::AddTo { state, .. } => {
                let len = self
                    .playlists
                    .iter()
                    .filter(|p| p.id != LIKED_PLAYLIST_ID)
                    .count();
                match key.code {
                    KeyCode::Esc | KeyCode::Char('q') => self.mode = Mode::Normal,
                    KeyCode::Up | KeyCode::Char('k') => state.select_previous(),
                    KeyCode::Down | KeyCode::Char('j') => {
                        let next = state
                            .selected()
                            .map_or(0, |i| (i + 1).min(len.saturating_sub(1)));
                        state.select(Some(next));
                    }
                    KeyCode::Enter => self.confirm_add_to(),
                    _ => {}
                }
                return;
            }
            Mode::Normal => {}
        }
        if let Some(action) = ui::keymap::action_for(key) {
            self.on_action(action);
        }
    }

    fn on_action(&mut self, action: Action) {
        match action {
            Action::Quit => self.quit = true,
            Action::Help => self.mode = Mode::Help,
            Action::Up => self.move_selection(-1),
            Action::Down => self.move_selection(1),
            Action::PageUp => self.move_selection(-10),
            Action::PageDown => self.move_selection(10),
            Action::Top => self.move_selection(i64::MIN / 2),
            Action::Bottom => self.move_selection(i64::MAX / 2),
            Action::FocusNext => {
                self.focus = match self.focus {
                    Focus::Playlists => Focus::Tracks,
                    Focus::Tracks => Focus::Playlists,
                }
            }
            Action::Select => self.select(),
            Action::TogglePause => self.player.toggle_pause(),
            Action::Next => self.skip(1),
            Action::Prev => self.skip(-1),
            Action::SeekBack => self.seek_by(-SEEK_STEP_SECS),
            Action::SeekForward => self.seek_by(SEEK_STEP_SECS),
            Action::VolumeUp => self.player.change_volume(VOLUME_STEP),
            Action::VolumeDown => self.player.change_volume(-VOLUME_STEP),
            Action::Sync => self.start_sync(),
            Action::Search => {
                self.view = TracksView::Playlist;
                self.mode = Mode::Search;
            }
            Action::Shuffle => {
                self.queue.toggle_shuffle();
                let state = if self.queue.shuffle { "on" } else { "off" };
                self.set_info(format!("Shuffle {state}"));
                self.prefetch_next();
            }
            Action::Repeat => {
                self.queue.repeat = self.queue.repeat.cycle();
                let label = match self.queue.repeat {
                    Repeat::Off => "off",
                    Repeat::All => "all",
                    Repeat::One => "one",
                };
                self.set_info(format!("Repeat {label}"));
            }
            Action::ToggleQueue => {
                self.view = match self.view {
                    TracksView::Playlist => TracksView::Queue,
                    TracksView::Queue => TracksView::Playlist,
                };
                self.focus = Focus::Tracks;
            }
            Action::PlayNext => {
                if let Some(track) = self.selected_track().cloned() {
                    self.set_info(format!("Playing next: {}", track.title));
                    self.queue.play_next(track);
                    self.prefetch_next();
                }
            }
            Action::Like => self.toggle_like(),
            Action::AddToPlaylist => self.open_add_to(),
        }
    }

    fn on_mouse(&mut self, mouse: MouseEvent) {
        tracing::trace!(?mouse, progress = ?self.areas.progress, "mouse");
        if !matches!(self.mode, Mode::Normal) {
            return;
        }
        let (x, y) = (mouse.column, mouse.row);
        let hit = |r: Rect| x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height;
        let Areas {
            playlists,
            tracks,
            progress,
        } = self.areas;
        match mouse.kind {
            MouseEventKind::ScrollDown | MouseEventKind::ScrollUp => {
                let delta = if mouse.kind == MouseEventKind::ScrollDown {
                    3
                } else {
                    -3
                };
                if hit(playlists) {
                    self.focus = Focus::Playlists;
                } else if hit(tracks) {
                    self.focus = Focus::Tracks;
                } else {
                    return;
                }
                self.move_selection(delta);
            }
            MouseEventKind::Down(MouseButton::Left) if hit(progress) => {
                if let Some(d) = self.player_status.duration {
                    let ratio = f64::from(x - progress.x) / f64::from(progress.width.max(1));
                    self.player.seek_to(d.mul_f64(ratio.clamp(0.0, 1.0)));
                    self.media_resync();
                }
            }
            MouseEventKind::Down(MouseButton::Left) if hit(playlists) => {
                self.focus = Focus::Playlists;
                // Row 0 is the border.
                let row = (y - playlists.y) as usize;
                if row >= 1 {
                    let index = self.playlist_state.offset() + row - 1;
                    if index < self.playlists.len() && Some(index) != self.playlist_state.selected()
                    {
                        self.playlist_state.select(Some(index));
                        self.load_tracks();
                    }
                }
            }
            MouseEventKind::Down(MouseButton::Left) if hit(tracks) => {
                self.focus = Focus::Tracks;
                // Border + header row, then tracks.
                let row = (y - tracks.y) as usize;
                if row >= 2 && self.view == TracksView::Playlist {
                    let index = self.track_offset + row - 2;
                    if index < self.visible.len() {
                        self.track_selected = Some(index);
                        let double = self
                            .last_click
                            .is_some_and(|(t, i)| i == index && t.elapsed() < DOUBLE_CLICK);
                        self.last_click = Some((Instant::now(), index));
                        if double {
                            self.play_selected();
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn on_media(&mut self, action: MediaAction) {
        match action {
            MediaAction::Toggle => self.player.toggle_pause(),
            MediaAction::Play => self.player.set_paused(false),
            MediaAction::Pause => self.player.set_paused(true),
            MediaAction::Next => self.skip(1),
            MediaAction::Prev => self.skip(-1),
            MediaAction::SeekBy(secs) => self.seek_by(secs),
            MediaAction::SeekTo(pos) => {
                self.player.seek_to(pos);
                self.media_resync();
            }
        }
    }

    // --- navigation ------------------------------------------------------------

    fn selected_playlist(&self) -> Option<&Playlist> {
        self.playlist_state
            .selected()
            .and_then(|i| self.playlists.get(i))
    }

    fn selected_track(&self) -> Option<&Track> {
        let row = self.track_selected?;
        self.tracks.get(*self.visible.get(row)? as usize)
    }

    fn move_selection(&mut self, delta: i64) {
        let clamp = |current: Option<usize>, len: usize| {
            (current.unwrap_or(0) as i64)
                .saturating_add(delta)
                .clamp(0, len as i64 - 1) as usize
        };
        match self.focus {
            Focus::Playlists if !self.playlists.is_empty() => {
                let target = clamp(self.playlist_state.selected(), self.playlists.len());
                if Some(target) != self.playlist_state.selected() {
                    self.playlist_state.select(Some(target));
                    self.load_tracks();
                }
            }
            Focus::Tracks if self.view == TracksView::Playlist && !self.visible.is_empty() => {
                self.track_selected = Some(clamp(self.track_selected, self.visible.len()));
            }
            _ => {}
        }
    }

    fn select(&mut self) {
        match self.focus {
            Focus::Playlists => {
                self.view = TracksView::Playlist;
                if !self.visible.is_empty() {
                    self.focus = Focus::Tracks;
                }
            }
            Focus::Tracks if self.view == TracksView::Playlist => self.play_selected(),
            Focus::Tracks => {}
        }
    }

    fn play_selected(&mut self) {
        let Some(&index) = self.track_selected.and_then(|row| self.visible.get(row)) else {
            return;
        };
        self.queue.set(self.tracks.clone(), index as usize);
        self.failures = 0;
        self.play_current();
    }

    fn skip(&mut self, direction: i32) {
        let moved = if direction > 0 {
            self.queue.advance().is_some()
        } else {
            // Like most players: "previous" restarts a track that's > 3 s in.
            if self.player_status.position > Duration::from_secs(3)
                && self.queue.current().is_some()
            {
                self.player.seek_to(Duration::ZERO);
                self.media_resync();
                return;
            }
            self.queue.back().is_some()
        };
        if moved {
            self.play_current();
        }
    }

    fn seek_by(&mut self, secs: i64) {
        self.player.seek_by(secs);
        self.media_resync();
    }

    /// Re-sends position to the OS widget after a jump.
    fn media_resync(&mut self) {
        if let Some(media) = &mut self.media {
            let s = self.player.status();
            media.set_state(s.state, s.position, true);
        }
    }

    fn reload_playlists(&mut self, prefer_id: Option<&str>) {
        let keep = prefer_id
            .map(str::to_owned)
            .or_else(|| self.selected_playlist().map(|p| p.id.clone()));
        match self.deps.library.playlists() {
            Ok(playlists) => self.playlists = playlists,
            Err(err) => self.set_error(format!("library: {err:#}")),
        }
        let index = keep
            .and_then(|id| self.playlists.iter().position(|p| p.id == id))
            .or((!self.playlists.is_empty()).then_some(0));
        self.playlist_state.select(index);
        self.load_tracks();
    }

    fn load_tracks(&mut self) {
        let Some(id) = self.selected_playlist().map(|p| p.id.clone()) else {
            self.tracks = Arc::from([]);
            self.apply_filter();
            return;
        };
        match self.deps.library.tracks(&id) {
            Ok(tracks) => self.tracks = tracks,
            Err(err) => self.set_error(format!("library: {err:#}")),
        }
        self.filter.clear();
        self.apply_filter();
    }

    /// Rebuilds `visible` from `filter` (case-insensitive title/artist match).
    fn apply_filter(&mut self) {
        let needle: Vec<char> = self.filter.chars().flat_map(char::to_lowercase).collect();
        self.visible.clear();
        self.visible.extend(
            self.tracks
                .iter()
                .enumerate()
                .filter(|(_, t)| {
                    needle.is_empty()
                        || contains_ci(&t.title, &needle)
                        || contains_ci(&t.artist, &needle)
                })
                .map(|(i, _)| i as u32),
        );
        self.visible.shrink_to_fit();
        self.track_selected = (!self.visible.is_empty()).then_some(0);
        self.track_offset = 0;
    }

    // --- playback ----------------------------------------------------------------

    fn play_current(&mut self) {
        let Some(track) = self.queue.current().cloned() else {
            return;
        };
        self.generation += 1;
        self.loading = true;
        self.player.stop();
        self.status = None;

        let generation = self.generation;
        let (resolver, http, tx) = (
            self.deps.resolver.clone(),
            self.deps.http.clone(),
            self.tx.clone(),
        );
        tokio::spawn(async move {
            let result = open_track(&resolver, &http, &track.video_id)
                .await
                .map(Box::new);
            let _ = tx.send(AppEvent::Opened { generation, result });
        });
    }

    fn prefetch_next(&self) {
        if let Some(next) = self.queue.peek_next() {
            self.deps.resolver.prefetch(&next.video_id);
        }
    }

    // --- library changes -----------------------------------------------------------

    fn youtube(&mut self) -> Option<Arc<YouTubeClient>> {
        if self.deps.youtube.is_none() {
            self.set_error("Not logged in: quit (q) and run `ytm login`");
        }
        self.deps.youtube.clone()
    }

    fn start_sync(&mut self) {
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
            let _ = tx.send(AppEvent::Synced(result));
        });
    }

    /// Target of like / add-to-playlist: the selected row in the track list,
    /// else the playing track.
    fn action_target(&self) -> Option<Track> {
        match (self.focus, self.view) {
            (Focus::Tracks, TracksView::Playlist) => self.selected_track().cloned(),
            _ => self.queue.current().cloned(),
        }
    }

    fn toggle_like(&mut self) {
        let Some(track) = self.action_target() else {
            return;
        };
        let Some(youtube) = self.youtube() else {
            return;
        };
        let liked = !self
            .deps
            .library
            .contains(LIKED_PLAYLIST_ID, &track.video_id)
            .unwrap_or(false);
        self.set_info(if liked {
            "Liking…"
        } else {
            "Removing like…"
        });
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = youtube.rate(&track.video_id, liked).await;
            let _ = tx.send(AppEvent::Rated {
                track,
                liked,
                result,
            });
        });
    }

    fn open_add_to(&mut self) {
        if self.youtube().is_none() {
            return;
        }
        let Some(track) = self.action_target() else {
            return;
        };
        if !self.playlists.iter().any(|p| p.id != LIKED_PLAYLIST_ID) {
            self.set_error("You have no playlists to add to");
            return;
        }
        let mut state = ListState::default();
        state.select(Some(0));
        self.mode = Mode::AddTo { track, state };
    }

    fn confirm_add_to(&mut self) {
        let Mode::AddTo { track, state } = std::mem::replace(&mut self.mode, Mode::Normal) else {
            return;
        };
        let Some(playlist) = state.selected().and_then(|i| {
            self.playlists
                .iter()
                .filter(|p| p.id != LIKED_PLAYLIST_ID)
                .nth(i)
                .cloned()
        }) else {
            return;
        };
        let Some(youtube) = self.youtube() else {
            return;
        };
        self.set_info(format!("Adding to {}…", playlist.title));
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = youtube.add_to_playlist(&playlist.id, &track.video_id).await;
            let _ = tx.send(AppEvent::Added {
                playlist,
                track,
                result,
            });
        });
    }

    // --- background results -----------------------------------------------------------

    fn on_app_event(&mut self, event: AppEvent) {
        match event {
            AppEvent::Synced(result) => {
                self.syncing = false;
                match result {
                    Ok(r) => {
                        self.reload_playlists(None);
                        self.set_info(format!(
                            "Synced {} playlists, {} tracks ({} unchanged, {} API units) · {}",
                            r.playlists,
                            r.tracks,
                            r.unchanged,
                            r.quota_units,
                            r.liked_note()
                        ));
                    }
                    Err(err) => self.set_error(format!("Sync failed: {err:#}")),
                }
            }
            AppEvent::Opened { generation, result } if generation == self.generation => {
                self.loading = false;
                match result {
                    Ok(opened) => {
                        let (body, stream) = *opened;
                        self.failures = 0;
                        let duration = stream.duration.or_else(|| {
                            self.queue
                                .current()
                                .and_then(|t| t.duration_secs)
                                .map(|s| Duration::from_secs(s.into()))
                        });
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
            AppEvent::Opened { .. } => {} // superseded by a newer selection
            AppEvent::Rated {
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
                    self.refresh_after_edit(LIKED_PLAYLIST_ID);
                    let verb = if liked { "♥ Liked" } else { "Removed like:" };
                    self.set_info(format!("{verb} {}", track.title));
                }
                Err(err) => self.set_error(format!("{err:#}")),
            },
            AppEvent::Added {
                playlist,
                track,
                result,
            } => match result {
                Ok(()) => {
                    if let Err(err) = self.deps.library.add_track(&playlist.id, &track, false) {
                        tracing::warn!(%err, "updating local playlist");
                    }
                    self.refresh_after_edit(&playlist.id);
                    self.set_info(format!("Added {} to {}", track.title, playlist.title));
                }
                Err(err) => self.set_error(format!("{err:#}")),
            },
        }
    }

    /// Reloads counts, and the track list if it shows the edited playlist.
    fn refresh_after_edit(&mut self, playlist_id: &str) {
        let viewing = self
            .selected_playlist()
            .is_some_and(|p| p.id == playlist_id);
        if let Ok(playlists) = self.deps.library.playlists() {
            self.playlists = playlists;
        }
        if viewing {
            let (filter, selected) = (std::mem::take(&mut self.filter), self.track_selected);
            if let Ok(tracks) = self.deps.library.tracks(playlist_id) {
                self.tracks = tracks;
            }
            self.filter = filter;
            self.apply_filter();
            if let Some(s) = selected.filter(|&s| s < self.visible.len()) {
                self.track_selected = Some(s);
            }
        }
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
            } if generation == self.generation => {
                self.track_failed(message);
            }
            _ => {}
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

    fn set_info(&mut self, text: impl Into<String>) {
        self.status = Some(Status {
            text: text.into(),
            is_error: false,
        });
    }

    fn set_error(&mut self, text: impl Into<String>) {
        let text = text.into();
        tracing::warn!("{text}");
        self.status = Some(Status {
            text,
            is_error: true,
        });
    }
}

/// Case-insensitive substring test without allocating (`needle` lowercased).
fn contains_ci(haystack: &str, needle: &[char]) -> bool {
    needle.is_empty()
        || haystack.char_indices().any(|(start, _)| {
            let mut chars = haystack[start..].chars().flat_map(char::to_lowercase);
            needle.iter().all(|n| chars.next() == Some(*n))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn case_insensitive_contains() {
        let needle: Vec<char> = "QUEEN".chars().flat_map(char::to_lowercase).collect();
        assert!(contains_ci("We are Queen fans", &needle));
        assert!(!contains_ci("Que", &needle));
        let cyr: Vec<char> = "ОКЕАН".chars().flat_map(char::to_lowercase).collect();
        assert!(contains_ci("Океан Ельзи", &cyr));
    }
}

//! Application root: state, the main `tokio::select!` loop (keys, background
//! results, player events, redraw tick) and the actions behind each key.

use std::{sync::Arc, time::Duration};

use anyhow::Result;
use crossterm::event::{Event, EventStream, KeyEventKind};
use futures_util::StreamExt;
use ratatui::{
    DefaultTerminal,
    widgets::{ListState, TableState},
};
use tokio::sync::mpsc::{self, UnboundedSender};

use crate::{
    api::{
        client::YouTubeClient,
        models::{Playlist, Track},
    },
    audio::{
        extractor::AudioStream,
        open_track,
        player::{PlayerEvent, PlayerHandle, PlayerStatus},
        queue::Queue,
        resolver::StreamResolver,
        stream::HttpStream,
    },
    storage::Library,
    sync::{SyncReport, sync_library},
    ui::{self, keymap::Action},
};

const SEEK_STEP_SECS: i64 = 5;
const VOLUME_STEP: f32 = 0.05;
/// Stop auto-skipping after this many tracks in a row fail to load.
const MAX_CONSECUTIVE_FAILURES: u32 = 3;

pub struct Deps {
    pub library: Arc<Library>,
    pub resolver: StreamResolver,
    pub http: reqwest::Client,
    /// `None` when not logged in.
    pub youtube: Option<Arc<YouTubeClient>>,
    pub liked_music_only: bool,
    pub volume: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Playlists,
    Tracks,
}

enum AppEvent {
    Synced(Result<SyncReport>),
    Opened {
        generation: u64,
        result: Result<Box<(HttpStream, AudioStream)>>,
    },
}

pub struct Status {
    pub text: String,
    pub is_error: bool,
}

pub struct App {
    pub(crate) playlists: Vec<Playlist>,
    pub(crate) tracks: Vec<Track>,
    pub(crate) focus: Focus,
    pub(crate) playlist_state: ListState,
    pub(crate) track_state: TableState,
    pub(crate) queue: Queue,
    /// Set while the current queue entry is being resolved/buffered.
    pub(crate) loading: bool,
    pub(crate) status: Option<Status>,
    pub(crate) syncing: bool,
    pub(crate) logged_in: bool,
    pub(crate) player_status: PlayerStatus,
    pub(crate) show_help: bool,
    generation: u64,
    failures: u32,
    quit: bool,
    player: PlayerHandle,
    deps: Deps,
    tx: UnboundedSender<AppEvent>,
}

pub async fn run(deps: Deps) -> Result<()> {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let (player_tx, mut player_rx) = mpsc::unbounded_channel();
    let player = PlayerHandle::spawn(deps.volume, player_tx)?;

    let mut app = App {
        playlists: Vec::new(),
        tracks: Vec::new(),
        focus: Focus::Playlists,
        playlist_state: ListState::default(),
        track_state: TableState::default(),
        queue: Queue::default(),
        loading: false,
        status: None,
        syncing: false,
        logged_in: deps.youtube.is_some(),
        player_status: player.status(),
        show_help: false,
        generation: 0,
        failures: 0,
        quit: false,
        player,
        deps,
        tx,
    };
    app.startup();

    // Restores the terminal on panic too (installs a panic hook).
    let mut terminal = ratatui::init();
    let result = app.event_loop(&mut terminal, &mut rx, &mut player_rx).await;
    ratatui::restore();
    result
}

impl App {
    fn startup(&mut self) {
        self.reload_playlists();
        let empty = self.playlists.is_empty();
        match (self.logged_in, empty) {
            (true, true) => self.start_sync(),
            (false, true) => self.set_error("Not logged in: quit (q) and run `ytm login`"),
            (false, false) => {
                self.set_info("Offline library (not logged in) — `ytm login` to sync")
            }
            (true, false) => {}
        }
    }

    async fn event_loop(
        &mut self,
        terminal: &mut DefaultTerminal,
        rx: &mut mpsc::UnboundedReceiver<AppEvent>,
        player_rx: &mut mpsc::UnboundedReceiver<PlayerEvent>,
    ) -> Result<()> {
        let mut keys = EventStream::new();
        // Redraw cadence for the progress bar; idle cost is one draw per tick.
        let mut tick = tokio::time::interval(Duration::from_millis(500));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        while !self.quit {
            self.player_status = self.player.status();
            terminal.draw(|frame| ui::draw(frame, self))?;

            tokio::select! {
                Some(event) = keys.next() => match event? {
                    // Windows also reports key releases.
                    Event::Key(key) if key.kind != KeyEventKind::Release => {
                        if let Some(action) = ui::keymap::action_for(key) {
                            self.on_action(action);
                        }
                    }
                    _ => {}
                },
                Some(event) = rx.recv() => self.on_app_event(event),
                Some(event) = player_rx.recv() => self.on_player_event(event),
                _ = tick.tick() => {}
            }
        }
        self.player.stop();
        Ok(())
    }

    fn on_action(&mut self, action: Action) {
        if self.show_help && action != Action::Quit {
            self.show_help = false;
            return;
        }
        match action {
            Action::Quit => self.quit = true,
            Action::Help => self.show_help = true,
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
            Action::Next => {
                if self.queue.advance().is_some() {
                    self.play_current();
                }
            }
            Action::Prev => {
                if self.queue.back().is_some() {
                    self.play_current();
                }
            }
            Action::SeekBack => self.player.seek_by(-SEEK_STEP_SECS),
            Action::SeekForward => self.player.seek_by(SEEK_STEP_SECS),
            Action::VolumeUp => self.player.change_volume(VOLUME_STEP),
            Action::VolumeDown => self.player.change_volume(-VOLUME_STEP),
            Action::Sync => self.start_sync(),
        }
    }

    fn move_selection(&mut self, delta: i64) {
        let (len, current) = match self.focus {
            Focus::Playlists => (self.playlists.len(), self.playlist_state.selected()),
            Focus::Tracks => (self.tracks.len(), self.track_state.selected()),
        };
        if len == 0 {
            return;
        }
        let target = (current.unwrap_or(0) as i64)
            .saturating_add(delta)
            .clamp(0, len as i64 - 1) as usize;
        match self.focus {
            Focus::Playlists => {
                if Some(target) != current {
                    self.playlist_state.select(Some(target));
                    self.load_tracks();
                }
            }
            Focus::Tracks => self.track_state.select(Some(target)),
        }
    }

    fn select(&mut self) {
        match self.focus {
            Focus::Playlists => {
                if !self.tracks.is_empty() {
                    self.focus = Focus::Tracks;
                }
            }
            Focus::Tracks => {
                if let Some(index) = self.track_state.selected() {
                    self.queue.set(self.tracks.clone(), index);
                    self.failures = 0;
                    self.play_current();
                }
            }
        }
    }

    fn reload_playlists(&mut self) {
        let selected_id = self
            .playlist_state
            .selected()
            .and_then(|i| self.playlists.get(i))
            .map(|p| p.id.clone());
        match self.deps.library.playlists() {
            Ok(playlists) => self.playlists = playlists,
            Err(err) => self.set_error(format!("library: {err:#}")),
        }
        let index = selected_id
            .and_then(|id| self.playlists.iter().position(|p| p.id == id))
            .or((!self.playlists.is_empty()).then_some(0));
        self.playlist_state.select(index);
        self.load_tracks();
    }

    fn load_tracks(&mut self) {
        let Some(playlist) = self
            .playlist_state
            .selected()
            .and_then(|i| self.playlists.get(i))
        else {
            self.tracks.clear();
            return;
        };
        match self.deps.library.tracks(&playlist.id) {
            Ok(tracks) => self.tracks = tracks,
            Err(err) => self.set_error(format!("library: {err:#}")),
        }
        self.track_state
            .select((!self.tracks.is_empty()).then_some(0));
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

    fn start_sync(&mut self) {
        let Some(youtube) = self.deps.youtube.clone() else {
            self.set_error("Not logged in: quit (q) and run `ytm login`");
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

    fn on_app_event(&mut self, event: AppEvent) {
        match event {
            AppEvent::Synced(result) => {
                self.syncing = false;
                match result {
                    Ok(r) => {
                        self.reload_playlists();
                        self.set_info(format!(
                            "Synced {} playlists, {} tracks ({} API units)",
                            r.playlists, r.tracks, r.quota_units
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
                        if let Some(next) = self.queue.peek_next() {
                            self.deps.resolver.prefetch(&next.video_id);
                        }
                    }
                    Err(err) => self.track_failed(format!("{err:#}")),
                }
            }
            AppEvent::Opened { .. } => {} // superseded by a newer selection
        }
    }

    fn on_player_event(&mut self, event: PlayerEvent) {
        match event {
            PlayerEvent::Ended { generation } if generation == self.generation => {
                if self.queue.advance().is_some() {
                    self.play_current();
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
            .map(|t| t.title.clone())
            .unwrap_or_default();
        self.set_error(format!("{title}: {message}"));
        if self.failures < MAX_CONSECUTIVE_FAILURES && self.queue.advance().is_some() {
            self.play_current();
            // Keep the error visible while the next track loads.
            self.set_error(format!("{title}: {message} — skipped"));
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

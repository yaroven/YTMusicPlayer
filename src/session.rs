//! UI-independent player core shared by the TUI and the GUI: library access,
//! queue and playback, background work (resolve, sync, likes, playlist edits)
//! and media-key handling. Frontends call the action methods and await
//! [`Session::next_event`] to learn what changed.

use std::{sync::Arc, time::Duration};

use anyhow::Result;
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
};

pub const SEEK_STEP_SECS: i64 = 5;
pub const VOLUME_STEP: f32 = 0.05;
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
    pub media_controls: bool,
    pub audio_device: Option<String>,
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
}

enum Background {
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

pub struct Session {
    deps: Deps,
    player: PlayerHandle,
    media: Option<MediaControls>,
    pub queue: Queue,
    /// Set while the current queue entry is being resolved/buffered.
    pub loading: bool,
    pub syncing: bool,
    pub logged_in: bool,
    pub status: Option<Status>,
    pub player_status: PlayerStatus,
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
        let (tx, rx) = mpsc::unbounded_channel();
        let (player_tx, player_rx) = mpsc::unbounded_channel();
        let (media_tx, media_rx) = mpsc::unbounded_channel();

        let lib = &deps.library;
        let meta = |key| lib.get_meta(key).ok().flatten();
        let volume = meta("volume")
            .and_then(|v| v.parse().ok())
            .unwrap_or(deps.volume);
        let mut queue = Queue::default();
        queue.shuffle = meta("shuffle").as_deref() == Some("1");
        queue.repeat = match meta("repeat").as_deref() {
            Some("all") => Repeat::All,
            Some("one") => Repeat::One,
            _ => Repeat::Off,
        };

        let player = PlayerHandle::spawn(volume, deps.audio_device.clone(), player_tx)?;
        let media = if deps.media_controls {
            MediaControls::new(media_tx)
        } else {
            None
        };
        Ok(Self {
            logged_in: deps.youtube.is_some(),
            player_status: player.status(),
            deps,
            player,
            media,
            queue,
            loading: false,
            syncing: false,
            status: None,
            generation: 0,
            failures: 0,
            tx,
            rx,
            player_rx,
            media_rx,
        })
    }

    pub fn library(&self) -> &Library {
        &self.deps.library
    }

    /// Kicks off the first sync, or explains why there's nothing to show.
    pub fn startup(&mut self, library_empty: bool) {
        match (self.logged_in, library_empty) {
            (true, true) => self.start_sync(),
            (false, true) => self.set_error("Not logged in: run `ytm login`"),
            (false, false) => {
                self.set_info("Offline library (not logged in) — `ytm login` to sync")
            }
            (true, false) => {}
        }
    }

    /// Persists volume, modes and the selected playlist.
    pub fn save_state(&self, selected_playlist: Option<&str>) {
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
        if let Some(id) = selected_playlist {
            pairs.push(("playlist", id));
        }
        for (key, value) in pairs {
            if let Err(err) = self.deps.library.set_meta(key, value) {
                tracing::warn!(%err, key, "saving state");
            }
        }
    }

    pub fn last_playlist(&self) -> Option<String> {
        self.deps.library.get_meta("playlist").ok().flatten()
    }

    pub fn shutdown(&mut self) {
        self.player.stop();
    }

    // --- events ---------------------------------------------------------------

    /// Re-reads player status (cheap); true when it changed.
    pub fn refresh_status(&mut self) -> bool {
        let status = self.player.status();
        if let Some(media) = &mut self.media {
            media.set_state(status.state, status.position, false);
        }
        let changed = status != self.player_status;
        self.player_status = status;
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
            Background::Opened { generation, result } if generation == self.generation => {
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
        let label = match self.queue.repeat {
            Repeat::Off => "off",
            Repeat::All => "all",
            Repeat::One => "one",
        };
        self.set_info(format!("Repeat {label}"));
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
        let (resolver, http, tx) = (
            self.deps.resolver.clone(),
            self.deps.http.clone(),
            self.tx.clone(),
        );
        tokio::spawn(async move {
            let result = open_track(&resolver, &http, &track.video_id)
                .await
                .map(Box::new);
            let _ = tx.send(Background::Opened { generation, result });
        });
    }

    fn prefetch_next(&self) {
        if let Some(next) = self.queue.peek_next() {
            self.deps.resolver.prefetch(&next.video_id);
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
        if self.deps.youtube.is_none() {
            self.set_error("Not logged in: run `ytm login`");
        }
        self.deps.youtube.clone()
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

//! Test support shared by the session, TUI and GUI tests: a recording
//! [`Playback`] fake, scripted extractors and a [`Session`] built on them
//! (no audio device, no network).

use std::{sync::Arc, time::Duration};

use tokio::sync::mpsc;

use crate::{
    account::Account,
    api::models::Track,
    audio::{
        TrackSource,
        player::{PlayState, Playback, PlayerEvent, PlayerStatus},
    },
    catalog::Catalog,
    session::{Deps, Session},
    storage::Library,
};
use std::sync::Mutex;

use futures_util::future::BoxFuture;
use tokio::sync::mpsc::UnboundedSender;

use crate::{
    api::token_store::{InMemory, Tokens},
    audio::{
        Extractor, JsPolicy,
        extractor::{self, AudioStream, ExtractorError},
        js_runtime::JsRuntime,
        player::Load,
    },
    config::settings::Settings,
};

/// Records calls; its status is set by the test.
#[derive(Clone)]
pub(crate) struct FakePlayer {
    pub(crate) calls: Arc<Mutex<Vec<String>>>,
    pub(crate) status: Arc<Mutex<PlayerStatus>>,
}

impl FakePlayer {
    pub(crate) fn new() -> Self {
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
    pub(crate) fn record(&self, call: String) {
        self.calls.lock().unwrap().push(call);
    }
    pub(crate) fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
    pub(crate) fn at(&self, position: Duration) {
        let mut s = self.status.lock().unwrap();
        s.state = PlayState::Playing;
        s.position = position;
    }
}

impl Playback for FakePlayer {
    fn load(&self, track: Load) {
        self.record(format!("load {}", track.generation));
    }
    fn preload(&self, track: Load) {
        self.record(format!("preload {}", track.generation));
    }
    fn cancel_preload(&self) {
        self.record("cancel_preload".into());
    }
    fn set_crossfade(&self, overlap: Duration) {
        self.record(format!("crossfade {}", overlap.as_secs()));
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
pub(crate) enum Resolves {
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
            Self::Fail => {
                Box::pin(
                    async move { Err(ExtractorError::Unavailable(format!("{video_id} is gone"))) },
                )
            }
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
    fn download<'a>(
        &'a self,
        _: &'a str,
        _: &'a std::path::Path,
    ) -> BoxFuture<'a, extractor::Result<()>> {
        Box::pin(async { Err(ExtractorError::NotManaged) })
    }
}

pub(crate) async fn session_with(
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
            Default::default(),
        ),
        catalog: Catalog::new(http.clone()),
        http,
        account,
        liked_music_only: true,
        volume: 0.5,
        media_controls: false,
        audio_device: None,
        autoplay: false,
        crossfade: Duration::ZERO,
        listeners: Vec::new(),
        download_limit: None,
        check_updates: false,
    };
    let player = FakePlayer::new();
    let (events, rx) = mpsc::unbounded_channel();
    let session = Session::with_playback(deps, Box::new(player.clone()), rx);
    (session, player, events)
}

pub(crate) async fn session(
    resolves: Resolves,
) -> (Session, FakePlayer, UnboundedSender<PlayerEvent>) {
    session_with(resolves, Arc::new(Library::open_in_memory().unwrap())).await
}

pub(crate) fn tracks(prefix: &str, n: usize) -> Arc<[Track]> {
    (0..n)
        .map(|i| Track {
            video_id: format!("{prefix}{i:0>10}").into(),
            title: format!("{prefix}{i}").into(),
            artist: "a".into(),
            duration_secs: Some(100),
        })
        .collect()
}

pub(crate) fn current(s: &Session) -> String {
    s.queue
        .current()
        .map_or("-".into(), |t| t.title.to_string())
}

/// Applies background results until nothing arrives for a while.
pub(crate) async fn settle(s: &mut Session) {
    while tokio::time::timeout(Duration::from_millis(300), s.next_event())
        .await
        .is_ok()
    {}
}

/// A library with Liked music (Queen, Ed Sheeran, Nirvana) and an empty
/// "Road trip" playlist.
pub(crate) fn seeded_library() -> Arc<Library> {
    use crate::{
        api::models::{LIKED_PLAYLIST_ID, Playlist},
        storage::SyncedPlaylist,
    };
    let song = |id: &str, title: &str, artist: &str, secs: u32| Track {
        video_id: id.into(),
        title: title.into(),
        artist: artist.into(),
        duration_secs: Some(secs),
    };
    let liked: Arc<[Track]> = Arc::from([
        song("fJ9rUzIMcZQ", "Bohemian Rhapsody", "Queen", 359),
        song("JGwWNGJdvx8", "Shape of You", "Ed Sheeran", 263),
        song("hTWKbfoikeg", "Smells Like Teen Spirit", "Nirvana", 278),
    ]);
    let playlist = |id: &str, title: &str, tracks: Arc<[Track]>| SyncedPlaylist {
        playlist: Playlist {
            id: id.into(),
            title: title.into(),
            item_count: tracks.len() as u32,
            etag: None,
        },
        item_ids: tracks
            .iter()
            .map(|t| format!("item-{}", t.video_id))
            .collect(),
        tracks,
    };
    let library = Library::open_in_memory().unwrap();
    library
        .replace_synced(&[
            playlist(LIKED_PLAYLIST_ID, "Liked music", liked),
            playlist("PLroadtrip", "Road trip", Arc::from([])),
        ])
        .unwrap();
    Arc::new(library)
}

/// An artist page: `songs` top songs and an albums shelf.
pub(crate) fn artist_page(songs: usize) -> crate::catalog::Page {
    use crate::catalog::{Item, ItemKind, Page, Shelf};
    Page {
        kind: ItemKind::Artist,
        id: "UCartist000000000000000".into(),
        title: "Test Artist".into(),
        subtitle: "1M monthly audience".into(),
        thumbnail: None,
        tracks: (0..songs)
            .map(|i| Track {
                video_id: format!("song{i:0>7}").into(),
                title: format!("Hit number {i}").into(),
                artist: "Test Artist".into(),
                duration_secs: Some(200 + i as u32),
            })
            .collect(),
        shelves: vec![Shelf {
            title: "Albums".into(),
            items: vec![Item {
                kind: ItemKind::Album,
                id: "MPREb_album1".into(),
                title: "First Album".into(),
                subtitle: "Album • 2020".into(),
                thumbnail: None,
                track: None,
            }],
        }],
        channel_id: Some("UCchannel".into()),
    }
}

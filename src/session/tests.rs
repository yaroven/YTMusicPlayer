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
        player::Load,
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

#[tokio::test]
async fn preloaded_track_takes_over_without_reloading() {
    let (mut s, player, events) = session(Resolves::Hang).await;
    s.play(tracks("t", 3), 0);
    // As if maybe_preload had opened the next track as load 9.
    s.preload = Some(9);
    events.send(PlayerEvent::Started { generation: 9 }).unwrap();
    s.next_event().await;
    assert_eq!(current(&s), "t1");
    assert_eq!(s.generation, 9);
    assert!(!player.calls().iter().any(|c| c == "load 9"), "no reload");
}

#[tokio::test]
async fn editing_the_queue_drops_the_preload() {
    let (mut s, player, _) = session(Resolves::Hang).await;
    s.play(tracks("t", 3), 0);
    s.preload = Some(9);
    s.remove_from_queue(0);
    assert!(s.preload.is_none());
    assert!(player.calls().contains(&"cancel_preload".to_owned()));
    assert_eq!(
        s.queue
            .upcoming()
            .map(|t| t.title.to_string())
            .collect::<Vec<_>>(),
        ["t2"]
    );
}

#[tokio::test]
async fn sleep_at_end_of_track_stops_instead_of_advancing() {
    let (mut s, _, events) = session(Resolves::Hang).await;
    s.play(tracks("t", 3), 0);
    s.set_sleep(None);
    events
        .send(PlayerEvent::Ended {
            generation: s.generation,
        })
        .unwrap();
    s.next_event().await;
    assert_eq!(current(&s), "t0", "stayed on the finished track");
    assert!(s.sleep.is_none());
}

#[tokio::test]
async fn plays_are_recorded_in_history() {
    let (mut s, player, _) = session(Resolves::Hang).await;
    s.play(tracks("t", 2), 1);
    player.at(Duration::from_secs(1));
    s.track_started(None);
    let history = s.library().history().unwrap();
    assert_eq!(history[0].title.as_ref(), "t1");
}

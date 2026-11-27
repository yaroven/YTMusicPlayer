//! The bridge between the window and the player core: the [`Cmd`]s the
//! UI thread sends, the [`Snapshot`]s the core thread pushes back, and the
//! core loop that turns one into the other (plus album-art fetches, which
//! only need the HTTP client).

use super::*;

pub(super) enum Cmd {
    Play(Arc<[Track]>, usize),
    ShufflePlay(Arc<[Track]>),
    JumpTo(usize),
    TogglePause,
    Next,
    Prev,
    SeekRatio(f64),
    SeekTo(Duration),
    Volume(f32),
    Shuffle,
    Repeat,
    Like(Track),
    Dislike(Track),
    PlayNext(Track),
    AddToQueue(Vec<Track>),
    Radio(Track),
    AddTo(Playlist, Track),
    RemoveFrom(Playlist, Track),
    CreatePlaylist(String, Option<Track>),
    RenamePlaylist(Playlist, String),
    DeletePlaylist(Playlist),
    ToggleSaved(Item, Option<String>),
    Download(Vec<Track>),
    RemoveDownload(Arc<str>),
    SaveQueue(String),
    MoveInPlaylist(Playlist, usize, usize),
    RemoveFromHistory(Arc<str>),
    ClearHistory,
    /// Check the connection, load again what failed for want of it.
    Retry,
    /// Switch the window to the video: the audio pauses.
    #[cfg(any(target_os = "macos", windows))]
    VideoOn,
    /// Back to audio, at the second the video stopped (when it reported one).
    #[cfg(any(target_os = "macos", windows))]
    VideoOff(Option<Duration>),
    SetLanguage(String),
    /// No web view here (Linux): open the clip in the browser.
    #[cfg(not(any(target_os = "macos", windows)))]
    WatchVideo,
    StopDownloads,
    RemoveAllDownloads,
    Sync,
    Search(String, SearchKind),
    OpenPage(String),
    /// A card was pressed: load its page ahead of the click.
    Prefetch(String),
    LoadHome,
    /// Home for a mood chip (its params), or the whole Home again.
    HomeChip(Option<String>),
    /// Home's next shelves (the list neared its end).
    MoreHome,
    LoadExplore,
    /// What is being typed in the search box (for suggestions).
    Suggest(String),
    QueueRemove(usize),
    QueueMove(usize, usize),
    ClearQueue,
    ToggleAutoplay,
    /// Minutes, [`SLEEP_END_OF_TRACK`] or [`SLEEP_OFF`].
    Sleep(i32),
    /// Which now-playing tabs are on screen: lyrics, related.
    Tabs(bool, bool),
    Crossfade(u64),
    Normalize(bool),
    StoreSetting(&'static str, bool),
    CastScan,
    CookiesBrowser(String),
    CheckUpdates,
    InstallUpdate,
    OpenArtist(Track),
    CastTo(Option<usize>),
    SaveClient(String, String),
    ImportClient(std::path::PathBuf),
    SignIn,
    SignOut,
    /// Sign in to YouTube: Google's page in a window, or (`Some`) a
    /// browser's sign-in.
    YouTubeSignIn(Option<String>),
    YouTubeSignOut,
    /// Art for a video id (or, for cards, an image URL) at `edge` px; the
    /// big cover's album art URL when the catalog has one.
    FetchArt(Arc<str>, ArtSize, u32, Option<Arc<str>>),
    /// Save state and stop.
    Quit,
}

/// What the core pushes to the UI thread.
pub(super) struct Snapshot {
    pub(super) view: SessionView,
    pub(super) changes: Changes,
    /// The latest search, when `changes.search`.
    pub(super) search: Option<SearchResults>,
    /// The opened page, when `changes.page`.
    pub(super) page: Option<Arc<Page>>,
    /// The home feed, when `changes.home`.
    pub(super) home: Option<Arc<[Shelf]>>,
    /// Explore shelves, when `changes.explore`.
    pub(super) explore: Option<Arc<[Shelf]>>,
    /// Home's mood chips and the chosen one, when `changes.home`.
    pub(super) home_chips: Option<(Arc<[HomeChip]>, Option<String>)>,
    pub(super) home_has_more: bool,
    /// Search suggestions, when `changes.suggestions`.
    pub(super) suggestions: Option<Arc<[String]>>,
    /// The playing track's lyrics, when `changes.lyrics`.
    pub(super) lyrics: Option<Arc<Lyrics>>,
    /// Related shelves, when `changes.related`.
    pub(super) related: Option<Arc<[Shelf]>>,
    /// Current track followed by what's next; only when it changed.
    pub(super) queue: Option<Arc<[Track]>>,
    /// Device names, when `changes.cast`.
    pub(super) cast_devices: Option<Vec<String>>,
    /// What installing an update did, and the release page.
    pub(super) update_done: Option<(crate::update::Installed, String)>,
    pub(super) cast_scanning: bool,
}

impl Snapshot {
    pub(super) fn of(session: &Session, changes: Changes, queue: Option<Arc<[Track]>>) -> Self {
        Self {
            view: session.view(),
            search: session.search.clone().filter(|_| changes.search),
            page: session.page.clone().filter(|_| changes.page),
            home: session.home.clone().filter(|_| changes.home),
            explore: session.explore.clone().filter(|_| changes.explore),
            home_chips: changes
                .home
                .then(|| (session.home_chips.clone(), session.home_chip.clone())),
            home_has_more: session.home_has_more(),
            suggestions: changes.suggestions.then(|| session.suggestions.clone()),
            lyrics: session
                .lyrics
                .as_ref()
                .and_then(|(_, l)| l.clone())
                .filter(|_| changes.lyrics),
            related: session
                .related
                .as_ref()
                .map(|(_, r)| r.clone())
                .filter(|_| changes.related),
            cast_devices: changes.cast.then(|| {
                session
                    .cast_devices
                    .iter()
                    .map(|d| d.name.clone())
                    .collect()
            }),
            cast_scanning: session.cast_scanning,
            update_done: None,
            changes,
            queue,
        }
    }
}

pub(super) async fn core_loop(
    deps: Deps,
    mut cmd_rx: UnboundedReceiver<Cmd>,
    library_empty: bool,
) -> Result<()> {
    let http = deps.http.clone();
    let fetches = Arc::new(Semaphore::new(ART_FETCHES));
    let mut session = Session::new(deps)?;
    session.startup(library_empty);
    // Memory measurements without a hand on the mouse (see CLAUDE.md):
    // play the restored queue's song as soon as the window is up.
    if std::env::var_os("YTM_PLAY_ON_START").is_some() {
        session.toggle_pause();
    }
    let mut queue_revision = None;
    let queue = queue_update(&session, &mut queue_revision);
    push(Snapshot::of(&session, Changes::default(), queue));

    let mut tick = tokio::time::interval(Duration::from_millis(500));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        let mut changes = Changes::default();
        let mut dirty = false;
        tokio::select! {
            cmd = cmd_rx.recv() => {
                dirty = true;
                match cmd {
                    None => break,
                    Some(Cmd::Quit) => {
                        session.save_state();
                        break;
                    }
                    Some(Cmd::FetchArt(id, size, edge, cover)) => {
                        dirty = false;
                        fetch_art(&http, &fetches, id, size, edge, cover);
                    }
                    Some(cmd) => apply(&mut session, cmd),
                }
            }
            c = session.next_event() => {
                changes = c;
                dirty = true;
            }
            _ = tick.tick() => {}
        }
        dirty |= session.refresh_status();
        let queue = queue_update(&session, &mut queue_revision);
        if dirty || queue.is_some() {
            let mut snap = Snapshot::of(&session, changes, queue);
            if let Some(done) = session.update_done.take() {
                let page = session.update.as_ref().map(|u| u.page.clone());
                snap.update_done = Some((done, page.unwrap_or_default()));
            }
            push(snap);
        }
    }
    session.shutdown();
    Ok(())
}

/// The "Up next" rows when the queue changed since `revision`.
pub(super) fn queue_update(session: &Session, revision: &mut Option<u64>) -> Option<Arc<[Track]>> {
    let q = &session.queue;
    if *revision == Some(q.revision()) {
        return None;
    }
    *revision = Some(q.revision());
    Some(
        q.current()
            .into_iter()
            .chain(q.upcoming().take(QUEUE_ROWS))
            .cloned()
            .collect(),
    )
}

/// Downloads and decodes art off the UI thread, then hands it over.
pub(super) fn fetch_art(
    http: &reqwest::Client,
    permits: &Arc<Semaphore>,
    id: Arc<str>,
    size: ArtSize,
    edge: u32,
    cover: Option<Arc<str>>,
) {
    let (http, permits) = (http.clone(), permits.clone());
    tokio::spawn(async move {
        let _permit = permits.acquire_owned().await;
        match art::fetch(&http, &id, size, edge, cover.as_deref()).await {
            Ok(art) => {
                let _ = slint::invoke_from_event_loop(move || {
                    let ui = current_ui();
                    VIEW.with(|cell| {
                        if let Some(view) = cell.borrow().clone() {
                            view.borrow_mut().art_arrived(ui.as_ref(), id, size, art);
                        }
                    });
                });
            }
            // Stays in `requested`, so it isn't retried every repaint.
            Err(err) => tracing::debug!(%err, %id, "album art"),
        }
    });
}

pub(super) fn apply(session: &mut Session, cmd: Cmd) {
    match cmd {
        Cmd::Play(tracks, index) => session.play(tracks, index),
        Cmd::ShufflePlay(tracks) => session.play_shuffled(tracks),
        Cmd::JumpTo(n) => session.skip_ahead(n),
        Cmd::TogglePause => session.toggle_pause(),
        Cmd::Next => session.skip(1),
        Cmd::Prev => session.skip(-1),
        Cmd::SeekRatio(r) => session.seek_ratio(r),
        Cmd::SeekTo(at) => session.seek_to(at),
        Cmd::Volume(v) => session.set_volume(v),
        Cmd::Shuffle => session.toggle_shuffle(),
        Cmd::Repeat => session.cycle_repeat(),
        Cmd::Like(track) => session.toggle_like(track),
        Cmd::Dislike(track) => session.dislike(track),
        Cmd::PlayNext(track) => session.play_next(track),
        Cmd::AddToQueue(tracks) => session.add_to_queue(tracks),
        Cmd::Radio(track) => session.start_radio(track),
        Cmd::AddTo(playlist, track) => session.add_to_playlist(playlist, track),
        Cmd::RemoveFrom(playlist, track) => session.remove_from_playlist(playlist, track),
        Cmd::CreatePlaylist(title, track) => session.create_playlist(&title, track),
        Cmd::RenamePlaylist(playlist, title) => session.rename_playlist(playlist, &title),
        Cmd::DeletePlaylist(playlist) => session.delete_playlist(playlist),
        Cmd::ToggleSaved(item, channel) => session.toggle_saved(item, channel),
        Cmd::Download(tracks) => session.download(tracks),
        Cmd::RemoveDownload(id) => session.remove_download(id),
        Cmd::SaveQueue(title) => session.save_queue_as_playlist(&title),
        Cmd::MoveInPlaylist(playlist, from, to) => session.move_in_playlist(playlist, from, to),
        Cmd::RemoveFromHistory(id) => session.remove_from_history(id),
        Cmd::ClearHistory => session.clear_history(),
        Cmd::Retry => session.retry(),
        #[cfg(any(target_os = "macos", windows))]
        Cmd::VideoOn => session.pause_for_video(),
        #[cfg(any(target_os = "macos", windows))]
        Cmd::VideoOff(at) => session.resume_from_video(at),
        Cmd::SetLanguage(code) => session.set_language(&code),
        #[cfg(not(any(target_os = "macos", windows)))]
        Cmd::WatchVideo => session.watch_video(),
        Cmd::StopDownloads => session.cancel_downloads(),
        Cmd::RemoveAllDownloads => session.remove_all_downloads(),
        Cmd::Sync => session.start_sync(),
        Cmd::Search(query, kind) => session.search(&query, kind),
        Cmd::OpenPage(id) => session.open_page(&id),
        Cmd::Prefetch(id) => session.prefetch_page(&id),
        Cmd::LoadHome => session.load_home(false),
        Cmd::HomeChip(params) => session.select_home_chip(params),
        Cmd::MoreHome => session.load_more_home(),
        Cmd::LoadExplore => session.load_explore(false),
        Cmd::Suggest(query) => session.suggest(&query),
        Cmd::QueueRemove(n) => session.remove_from_queue(n),
        Cmd::QueueMove(from, to) => session.move_in_queue(from, to),
        Cmd::ClearQueue => session.clear_queue(),
        Cmd::ToggleAutoplay => session.toggle_autoplay(),
        Cmd::Sleep(SLEEP_OFF) => session.cancel_sleep(),
        Cmd::Sleep(SLEEP_END_OF_TRACK) => session.set_sleep(None),
        Cmd::Sleep(minutes) => session.set_sleep(u32::try_from(minutes).ok()),
        Cmd::Tabs(lyrics, related) => {
            session.want_lyrics(lyrics);
            session.want_related(related);
        }
        Cmd::Crossfade(secs) => session.set_crossfade(Duration::from_secs(secs)),
        Cmd::Normalize(on) => session.set_normalize(on),
        Cmd::StoreSetting(key, on) => session.store_setting(key, &on.to_string()),
        Cmd::CastScan => session.find_cast_devices(),
        Cmd::CookiesBrowser(browser) => session.set_cookies_browser(&browser),
        Cmd::CheckUpdates => session.check_for_update(true),
        Cmd::InstallUpdate => session.install_update(),
        Cmd::OpenArtist(track) => session.open_artist(&track),
        Cmd::CastTo(i) => {
            let device = i.and_then(|i| session.cast_devices.get(i).cloned());
            session.cast_to(device);
        }
        Cmd::SaveClient(id, secret) => {
            session.set_client(&id, &secret);
        }
        Cmd::ImportClient(path) => {
            session.import_client(&path);
        }
        Cmd::SignIn => session.sign_in(),
        Cmd::YouTubeSignIn(browser) => session.sign_in_youtube(browser),
        Cmd::YouTubeSignOut => session.sign_out_youtube(),
        Cmd::SignOut => session.sign_out(),
        Cmd::FetchArt(..) | Cmd::Quit => {}
    }
}

/// Applies a snapshot on the UI thread (to the window, if one is open).
pub(super) fn push(snap: Snapshot) {
    let _ = slint::invoke_from_event_loop(move || {
        let ui = current_ui();
        VIEW.with(|cell| {
            if let Some(view) = cell.borrow().clone() {
                view.borrow_mut().absorb(snap, ui.as_ref());
            }
        });
    });
}

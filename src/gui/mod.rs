//! Desktop frontend (Slint, software renderer).
//!
//! Threads: the Slint event loop owns the main thread (on macOS that is also
//! where media-key callbacks arrive); the [`Session`] runs on a "core" thread
//! with its own tokio runtime. The UI sends [`Cmd`]s; the core pushes a
//! [`Snapshot`] back whenever player state changes. Library reads (playlists,
//! tracks) happen on the UI thread straight from SQLite.
//!
//! Album art is fetched lazily for rows the list actually shows and kept in
//! a small bounded cache ([`ArtCache`]); only the playing track gets a large
//! cover.

mod art;
mod ui;

use std::{
    cell::RefCell,
    collections::{HashMap, HashSet, VecDeque},
    rc::Rc,
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::{Context, Result, anyhow};
use slint::{
    Color, ComponentHandle, Image, Model, ModelNotify, ModelRc, ModelTracker, SharedString,
    VecModel,
};
use tokio::sync::{
    Semaphore,
    mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel},
};

use crate::{
    account,
    api::models::{Playlist, Track},
    audio::{player::PlayState, queue::Repeat},
    catalog::SearchKind,
    fmt::fmt_time,
    library_view::{LibraryView, SessionData},
    session::{Changes, Deps, SearchResults, Session, SessionView},
    storage::Library,
};
use art::ArtSize;
use ui::{MainWindow, PlaylistRow, TrackRow};

/// Decoded thumbnails kept in memory (~36 KB each at 96x96 RGBA): two
/// screens of rows plus the queue panel on a tall window.
const THUMB_CACHE: usize = 80;
/// Rows of the "Up next" list sent to the UI.
const QUEUE_ROWS: usize = 200;
/// Parallel thumbnail downloads.
const ART_FETCHES: usize = 4;

enum Cmd {
    Play(Arc<[Track]>, usize),
    ShufflePlay(Arc<[Track]>),
    JumpTo(usize),
    TogglePause,
    Next,
    Prev,
    SeekRatio(f64),
    Volume(f32),
    Shuffle,
    Repeat,
    Like(Track),
    PlayNext(Track),
    AddTo(Playlist, Track),
    Sync,
    Search(String),
    SaveClient(String, String),
    ImportClient(std::path::PathBuf),
    SignIn,
    SignOut,
    FetchArt(Arc<str>, ArtSize),
    /// Save state and stop.
    Quit,
}

/// What the core pushes to the UI thread.
struct Snapshot {
    view: SessionView,
    changes: Changes,
    /// The latest search, when `changes.search`.
    search: Option<SearchResults>,
    /// Current track followed by what's next; only when it changed.
    queue: Option<Arc<[Track]>>,
}

impl Snapshot {
    fn of(session: &Session, changes: Changes, queue: Option<Arc<[Track]>>) -> Self {
        Self {
            view: session.view(),
            search: session.search.clone().filter(|_| changes.search),
            changes,
            queue,
        }
    }
}

/// The playing track's big cover: id, image, average colour.
type LargeArt = (Arc<str>, Image, (u8, u8, u8));

/// Bounded cache of decoded album art (UI thread only).
struct ArtCache {
    thumbs: HashMap<Arc<str>, Image>,
    order: VecDeque<Arc<str>>,
    /// Requested and not yet answered, or failed: never re-requested.
    requested: HashSet<Arc<str>>,
    /// The single large cover (for the playing track).
    large: Option<LargeArt>,
    tx: UnboundedSender<Cmd>,
}

impl ArtCache {
    fn thumb(&mut self, id: &Arc<str>) -> Option<Image> {
        if let Some(image) = self.thumbs.get(id) {
            return Some(image.clone());
        }
        if self.requested.insert(id.clone()) {
            let _ = self.tx.send(Cmd::FetchArt(id.clone(), ArtSize::Thumb));
        }
        None
    }

    fn insert_thumb(&mut self, id: Arc<str>, image: Image) {
        self.requested.remove(&id);
        if self.thumbs.insert(id.clone(), image).is_none() {
            self.order.push_back(id);
        }
        while self.order.len() > THUMB_CACHE {
            if let Some(old) = self.order.pop_front() {
                self.thumbs.remove(&old);
            }
        }
    }
}

/// Where a [`TracksModel`] gets its rows.
enum Rows {
    /// The library view's (filtered) list.
    Library(Rc<RefCell<LibraryView>>),
    /// The play queue: current track, then what's next.
    Queue(RefCell<Arc<[Track]>>),
}

/// Track rows for a virtualized ListView: built (and their art requested)
/// only when the list asks for them; no track is copied up front.
struct TracksModel {
    rows: Rows,
    playing: RefCell<Option<Arc<str>>>,
    art: Rc<RefCell<ArtCache>>,
    notify: ModelNotify,
}

impl TracksModel {
    fn new(rows: Rows, art: Rc<RefCell<ArtCache>>) -> Self {
        Self {
            rows,
            playing: RefCell::default(),
            art,
            notify: ModelNotify::default(),
        }
    }

    /// All rows changed (new list, new filter).
    fn reset(&self) {
        self.notify.reset();
    }

    fn set_queue(&self, tracks: Arc<[Track]>) {
        if let Rows::Queue(cell) = &self.rows {
            *cell.borrow_mut() = tracks;
            self.notify.reset();
        }
    }

    fn set_playing(&self, id: Option<Arc<str>>) {
        if *self.playing.borrow() == id {
            return;
        }
        let old = std::mem::replace(&mut *self.playing.borrow_mut(), id.clone());
        for changed in [old, id].into_iter().flatten() {
            self.rows_changed(&changed);
        }
    }

    /// Re-renders rows showing `id` (art arrived, like or playing changed).
    fn rows_changed(&self, id: &str) {
        let rows = match &self.rows {
            Rows::Library(view) => view.borrow().rows_of(id),
            Rows::Queue(tracks) => tracks
                .borrow()
                .iter()
                .enumerate()
                .filter(|(_, t)| &*t.video_id == id)
                .map(|(row, _)| row)
                .collect(),
        };
        for row in rows {
            self.notify.row_changed(row);
        }
    }
}

impl Model for TracksModel {
    type Data = TrackRow;

    fn row_count(&self) -> usize {
        match &self.rows {
            Rows::Library(view) => view.borrow().len(),
            Rows::Queue(tracks) => tracks.borrow().len(),
        }
    }

    fn row_data(&self, row: usize) -> Option<TrackRow> {
        let (t, liked) = match &self.rows {
            Rows::Library(view) => {
                let view = view.borrow();
                let (_, t) = view.row(row)?;
                (t.clone(), view.is_liked(&t.video_id))
            }
            // Queue rows show no like button.
            Rows::Queue(tracks) => (tracks.borrow().get(row)?.clone(), false),
        };
        let playing = self.playing.borrow().as_ref() == Some(&t.video_id);
        let art = self.art.borrow_mut().thumb(&t.video_id);
        Some(TrackRow {
            liked,
            initial: initial(&t.title),
            hue: hue(&t.artist),
            title: SharedString::from(&*t.title),
            artist: SharedString::from(&*t.artist),
            time: t
                .duration_secs
                .map(|s| fmt_time(Duration::from_secs(s.into())))
                .unwrap_or_default()
                .into(),
            playing,
            has_art: art.is_some(),
            art: art.unwrap_or_default(),
        })
    }

    fn model_tracker(&self) -> &dyn ModelTracker {
        &self.notify
    }
}

/// First letter or digit of a title, for the album-art stand-in.
fn initial(title: &str) -> SharedString {
    title
        .chars()
        .find(|c| c.is_alphanumeric())
        .map(|c| c.to_uppercase().collect::<String>())
        .unwrap_or_default()
        .into()
}

/// Stable 0..1 hue per artist, so one artist's tiles share a colour.
fn hue(artist: &str) -> f32 {
    let h = artist.bytes().fold(0x811c_9dc5_u32, |h, b| {
        (h ^ u32::from(b)).wrapping_mul(0x0100_0193)
    });
    (h % 360) as f32 / 360.0
}

/// UI-thread state. What the list shows is the [`LibraryView`]; this only
/// mirrors it into the window's properties and models.
struct View {
    library: Rc<RefCell<LibraryView>>,
    tracks: Rc<TracksModel>,
    queue: Rc<TracksModel>,
    playlist_rows: Rc<VecModel<PlaylistRow>>,
    /// "Save to playlist" choices (every playlist but Liked music).
    add_rows: Rc<VecModel<SharedString>>,
    art: Rc<RefCell<ArtCache>>,
    /// The playing track (from the last snapshot).
    now: Option<Track>,
}

impl View {
    /// Playlist names and counts changed (sidebar, "save to" choices).
    fn sync_playlists(&self) {
        let lib = self.library.borrow();
        let row = |p: &Playlist| PlaylistRow {
            title: p.title.as_str().into(),
            count: p.item_count as i32,
        };
        self.playlist_rows
            .set_vec(lib.playlists().iter().map(row).collect::<Vec<_>>());
        self.add_rows.set_vec(
            lib.add_choices()
                .map(|p| SharedString::from(p.title.as_str()))
                .collect::<Vec<_>>(),
        );
    }

    /// The list was replaced: refresh everything the window shows about it.
    fn sync_list(&self, ui: &MainWindow) {
        self.sync_playlists();
        let lib = self.library.borrow();
        ui.set_selected_playlist(lib.selected_playlist().map_or(-1, |i| i as i32));
        ui.set_showing_results(lib.showing_results());
        ui.set_search_query(lib.search_query().unwrap_or_default().into());
        ui.set_tracks_title(lib.title().into());
        ui.set_track_count(lib.total() as i32);
        ui.set_selected_track(lib.selected_row().map_or(-1, |r| r as i32));
        if lib.filter().is_empty() {
            // A new list starts unfiltered: empty the search field too.
            ui.set_clear_search(ui.get_clear_search().wrapping_add(1));
        }
        drop(lib);
        self.tracks.reset();
    }

    /// Takes the row the window marked as selected (rows set it themselves
    /// before "play next" / "save to playlist").
    fn selection_from(&self, ui: &MainWindow) {
        self.library
            .borrow_mut()
            .select_row(usize::try_from(ui.get_selected_track()).ok());
    }

    /// What an action applies to: the selected row, else the playing track.
    fn target(&self, ui: &MainWindow) -> Option<Track> {
        self.selection_from(ui);
        self.library.borrow().target(self.now.as_ref())
    }

    /// Shows the playing track's covers, requesting them if needed.
    fn show_now_art(&mut self, ui: &MainWindow) {
        let Some(id) = self.now.as_ref().map(|t| t.video_id.clone()) else {
            ui.set_now_has_art(false);
            ui.set_now_has_thumb(false);
            return;
        };
        let thumb = self.art.borrow_mut().thumb(&id);
        ui.set_now_has_thumb(thumb.is_some());
        ui.set_now_thumb(thumb.unwrap_or_default());

        let mut art = self.art.borrow_mut();
        match &art.large {
            Some((large_id, image, tint)) if *large_id == id => {
                ui.set_now_art(image.clone());
                ui.set_now_has_art(true);
                ui.set_now_tint(Color::from_rgb_u8(tint.0, tint.1, tint.2));
            }
            _ => {
                ui.set_now_has_art(false);
                ui.set_now_tint(Color::from_rgb_u8(0x30, 0x30, 0x30));
                // Drop the old cover right away: only one large image lives.
                art.large = None;
                let _ = art.tx.send(Cmd::FetchArt(id, ArtSize::Large));
            }
        }
    }

    /// Art for `id` arrived on the UI thread.
    fn art_arrived(&mut self, ui: &MainWindow, id: Arc<str>, size: ArtSize, art: art::Art) {
        let image = Image::from_rgba8(art.pixels);
        let is_now = self.now.as_ref().is_some_and(|t| t.video_id == id);
        match size {
            ArtSize::Thumb => {
                self.art.borrow_mut().insert_thumb(id.clone(), image);
                self.tracks.rows_changed(&id);
                self.queue.rows_changed(&id);
            }
            ArtSize::Large if is_now => {
                self.art.borrow_mut().large = Some((id.clone(), image, art.tint));
            }
            ArtSize::Large => return, // track changed meanwhile
        }
        if is_now {
            self.show_now_art(ui);
        }
    }
}

/// File name of a downloaded OAuth client JSON, for the import button.
fn downloaded_client_name() -> String {
    account::downloaded_client_json()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_default()
}

/// The open window, for [`raise`] (called from the single-instance thread).
static WINDOW: Mutex<Option<slint::Weak<MainWindow>>> = Mutex::new(None);

/// Brings the window to the front (another launch asked for it).
pub fn raise() {
    let window = WINDOW.lock().unwrap_or_else(|e| e.into_inner()).clone();
    if let Some(weak) = window {
        let _ = weak.upgrade_in_event_loop(|ui| {
            use slint::winit_030::WinitWindowAccessor;
            let window = ui.window();
            window.set_minimized(false);
            let _ = window.show();
            window.with_winit_window(|w| w.focus_window());
        });
    }
}

/// Runs the GUI until the window closes. `rt` runs the core on its own thread.
pub fn run(rt: tokio::runtime::Runtime, deps: Deps) -> Result<()> {
    let ui = MainWindow::new().context("cannot open a window (no display?)")?;
    *WINDOW.lock().unwrap_or_else(|e| e.into_inner()) = Some(ui.as_weak());
    let library = deps.library.clone();
    let library_view = Rc::new(RefCell::new(LibraryView::new(library.clone())?));
    let (cmd_tx, cmd_rx) = unbounded_channel();

    let art = Rc::new(RefCell::new(ArtCache {
        thumbs: HashMap::new(),
        order: VecDeque::new(),
        requested: HashSet::new(),
        large: None,
        tx: cmd_tx.clone(),
    }));
    let view = Rc::new(RefCell::new(View {
        tracks: Rc::new(TracksModel::new(
            Rows::Library(library_view.clone()),
            art.clone(),
        )),
        queue: Rc::new(TracksModel::new(
            Rows::Queue(RefCell::new(Arc::from([]))),
            art.clone(),
        )),
        library: library_view,
        playlist_rows: Rc::new(VecModel::default()),
        add_rows: Rc::new(VecModel::default()),
        art,
        now: None,
    }));
    VIEW.with(|cell| *cell.borrow_mut() = Some(view.clone()));
    {
        let v = view.borrow();
        ui.set_tracks(ModelRc::from(v.tracks.clone()));
        ui.set_queue_rows(ModelRc::from(v.queue.clone()));
        ui.set_playlists(ModelRc::from(v.playlist_rows.clone()));
        ui.set_add_choices(ModelRc::from(v.add_rows.clone()));
        v.sync_list(&ui);
    }
    if let Some(width) = library
        .get_meta("sidebar_width")
        .ok()
        .flatten()
        .and_then(|w| w.parse::<f32>().ok())
    {
        ui.set_sidebar_width(width);
    }
    let library_empty = view.borrow().library.borrow().playlists().is_empty();

    wire_callbacks(&ui, &view, &cmd_tx, &library);
    // First start: nothing works without an account, so ask right away.
    if !deps.account.signed_in() {
        ui.set_download_file(downloaded_client_name().into());
        ui.set_account_open(true);
    }

    // Core thread: session + background work. Pushes snapshots to the UI.
    let weak = ui.as_weak();
    let core = std::thread::Builder::new()
        .name("core".into())
        .spawn(move || rt.block_on(core_loop(deps, cmd_rx, weak, library_empty)))?;

    ui.run().context("GUI event loop failed")?;

    view.borrow().library.borrow().save();
    let _ = cmd_tx.send(Cmd::Quit);
    core.join().map_err(|_| anyhow!("core thread panicked"))?
}

fn wire_callbacks(
    ui: &MainWindow,
    view: &Rc<RefCell<View>>,
    cmd_tx: &UnboundedSender<Cmd>,
    library: &Arc<Library>,
) {
    let send = |cmd: Cmd, tx: &UnboundedSender<Cmd>| {
        let _ = tx.send(cmd);
    };
    // Each handler gets `tx`, `view` and `ui` (names passed in for hygiene).
    macro_rules! on {
        // `||` is a single token, so zero-argument closures need their own arm.
        ($setter:ident, [$tx:ident, $view:ident, $ui:ident], || $body:expr) => {
            on!($setter, [$tx, $view, $ui], | | $body)
        };
        ($setter:ident, [$tx:ident, $view:ident, $ui:ident], |$($arg:ident),*| $body:expr) => {{
            let ($tx, $view, weak) = (cmd_tx.clone(), view.clone(), ui.as_weak());
            ui.$setter(move |$($arg),*| {
                let Some($ui) = weak.upgrade() else { return };
                #[allow(unused_variables)]
                let ($tx, $view, $ui) = (&$tx, &$view, &$ui);
                $body
            });
        }};
    }

    // Starting playback from the list remembers where the queue came from.
    let play =
        |ui: &MainWindow, view: &RefCell<View>, cmd: Option<Cmd>, tx: &UnboundedSender<Cmd>| {
            if let Some(cmd) = cmd {
                ui.set_queue_source(view.borrow().library.borrow().source_name().into());
                let _ = tx.send(cmd);
            }
        };
    // Shows playlist `index` (or the search results), then mirrors it.
    let show = |ui: &MainWindow, view: &RefCell<View>, index: Option<usize>| {
        let v = view.borrow();
        let result = match index {
            Some(i) => v.library.borrow_mut().select_playlist(Some(i)),
            None => {
                v.library.borrow_mut().show_results();
                Ok(())
            }
        };
        if let Err(err) = result {
            tracing::warn!(%err, "loading playlist");
            ui.set_status_text(format!("library: {err:#}").into());
            ui.set_status_error(true);
        }
        v.sync_list(ui);
    };

    on!(on_select_playlist, [tx, view, ui], |index| {
        if let Ok(i) = usize::try_from(index) {
            show(ui, view, Some(i));
        }
    });
    on!(on_show_results, [tx, view, ui], || show(ui, view, None));
    on!(on_select_track, [tx, view, ui], |row| {
        ui.set_selected_track(row);
        view.borrow().selection_from(ui);
    });
    on!(on_play_track, [tx, view, ui], |row| {
        ui.set_selected_track(row);
        let cmd = {
            let v = view.borrow();
            v.selection_from(ui);
            let lib = v.library.borrow();
            lib.play_from(lib.selected_row())
                .map(|(tracks, index)| Cmd::Play(tracks, index))
        };
        play(ui, view, cmd, tx);
    });
    on!(on_play_all, [tx, view, ui], || {
        let cmd = view
            .borrow()
            .library
            .borrow()
            .play_from(None)
            .map(|(tracks, index)| Cmd::Play(tracks, index));
        play(ui, view, cmd, tx);
    });
    on!(on_shuffle_play, [tx, view, ui], || {
        let cmd = view
            .borrow()
            .library
            .borrow()
            .all_tracks()
            .map(Cmd::ShufflePlay);
        play(ui, view, cmd, tx);
    });
    on!(on_jump, [tx, view, ui], |row| {
        // Row 0 is the playing track itself.
        if let Some(n) = usize::try_from(row).ok().and_then(|r| r.checked_sub(1)) {
            send(Cmd::JumpTo(n), tx);
        }
    });
    on!(on_filter_changed, [tx, view, ui], |text| {
        let v = view.borrow();
        v.library.borrow_mut().set_filter(&text);
        ui.set_selected_track(-1);
        v.tracks.reset();
    });
    on!(on_toggle_pause, [tx, view, ui], || send(
        Cmd::TogglePause,
        tx
    ));
    on!(on_next, [tx, view, ui], || send(Cmd::Next, tx));
    on!(on_prev, [tx, view, ui], || send(Cmd::Prev, tx));
    on!(on_seek, [tx, view, ui], |ratio| send(
        Cmd::SeekRatio(ratio.into()),
        tx
    ));
    on!(on_volume_changed, [tx, view, ui], |v| send(
        Cmd::Volume(v),
        tx
    ));
    on!(on_toggle_shuffle, [tx, view, ui], || send(Cmd::Shuffle, tx));
    on!(on_cycle_repeat, [tx, view, ui], || send(Cmd::Repeat, tx));
    on!(on_sync, [tx, view, ui], || send(Cmd::Sync, tx));
    on!(on_toggle_like, [tx, view, ui], || {
        // The player bar's heart (and `f`) belongs to the playing track;
        // rows have their own hearts.
        let v = view.borrow();
        if let Some(track) = v.now.clone().or_else(|| v.target(ui)) {
            send(Cmd::Like(track), tx);
        }
    });
    on!(on_play_next, [tx, view, ui], || {
        if let Some(track) = view.borrow().target(ui) {
            send(Cmd::PlayNext(track), tx);
        }
    });
    on!(on_search_online, [tx, view, ui], |text| {
        let query = text.trim();
        if !query.is_empty() {
            send(Cmd::Search(query.to_owned()), tx);
        }
    });
    on!(on_like_row, [tx, view, ui], |row| {
        let track = usize::try_from(row).ok().and_then(|r| {
            view.borrow()
                .library
                .borrow()
                .row(r)
                .map(|(_, t)| t.clone())
        });
        if let Some(track) = track {
            send(Cmd::Like(track), tx);
        }
    });
    let library = library.clone();
    on!(on_sidebar_resized, [tx, view, ui], |width| {
        if let Err(err) = library.set_meta("sidebar_width", &format!("{width:.0}")) {
            tracing::warn!(%err, "saving sidebar width");
        }
    });
    on!(on_account_opened, [tx, view, ui], || ui
        .set_download_file(downloaded_client_name().into()));
    on!(on_save_client, [tx, view, ui], |id, secret| send(
        Cmd::SaveClient(id.trim().to_owned(), secret.trim().to_owned()),
        tx
    ));
    on!(on_import_downloaded, [tx, view, ui], || {
        if let Some(path) = account::downloaded_client_json() {
            send(Cmd::ImportClient(path), tx);
        }
    });
    on!(on_sign_in, [tx, view, ui], || send(Cmd::SignIn, tx));
    on!(on_sign_out, [tx, view, ui], || send(Cmd::SignOut, tx));
    on!(on_open_console, [tx, view, ui], || {
        let _ = open::that_detached(account::CONSOLE_URL);
    });
    on!(on_add_to, [tx, view, ui], |index| {
        let v = view.borrow();
        let playlist = usize::try_from(index)
            .ok()
            .and_then(|i| v.library.borrow().add_choice(i).cloned());
        if let (Some(playlist), Some(track)) = (playlist, v.target(ui)) {
            send(Cmd::AddTo(playlist, track), tx);
        }
    });
}

async fn core_loop(
    deps: Deps,
    mut cmd_rx: UnboundedReceiver<Cmd>,
    ui: slint::Weak<MainWindow>,
    library_empty: bool,
) -> Result<()> {
    let http = deps.http.clone();
    let fetches = Arc::new(Semaphore::new(ART_FETCHES));
    let mut session = Session::new(deps)?;
    session.startup(library_empty);
    let mut queue_revision = None;
    let queue = queue_update(&session, &mut queue_revision);
    push(&ui, Snapshot::of(&session, Changes::default(), queue));

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
                    Some(Cmd::FetchArt(id, size)) => {
                        dirty = false;
                        fetch_art(&http, &fetches, &ui, id, size);
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
            push(&ui, Snapshot::of(&session, changes, queue));
        }
    }
    session.shutdown();
    Ok(())
}

/// The "Up next" rows when the queue changed since `revision`.
fn queue_update(session: &Session, revision: &mut Option<u64>) -> Option<Arc<[Track]>> {
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
fn fetch_art(
    http: &reqwest::Client,
    permits: &Arc<Semaphore>,
    ui: &slint::Weak<MainWindow>,
    id: Arc<str>,
    size: ArtSize,
) {
    let (http, permits, ui) = (http.clone(), permits.clone(), ui.clone());
    tokio::spawn(async move {
        let _permit = permits.acquire_owned().await;
        match art::fetch(&http, &id, size).await {
            Ok(art) => {
                let _ = ui.upgrade_in_event_loop(move |ui| {
                    VIEW.with(|cell| {
                        if let Some(view) = cell.borrow().clone() {
                            view.borrow_mut().art_arrived(&ui, id, size, art);
                        }
                    });
                });
            }
            // Stays in `requested`, so it isn't retried every repaint.
            Err(err) => tracing::debug!(%err, %id, "album art"),
        }
    });
}

fn apply(session: &mut Session, cmd: Cmd) {
    match cmd {
        Cmd::Play(tracks, index) => session.play(tracks, index),
        Cmd::ShufflePlay(tracks) => session.play_shuffled(tracks),
        Cmd::JumpTo(n) => session.skip_ahead(n),
        Cmd::TogglePause => session.toggle_pause(),
        Cmd::Next => session.skip(1),
        Cmd::Prev => session.skip(-1),
        Cmd::SeekRatio(r) => session.seek_ratio(r),
        Cmd::Volume(v) => session.set_volume(v),
        Cmd::Shuffle => session.toggle_shuffle(),
        Cmd::Repeat => session.cycle_repeat(),
        Cmd::Like(track) => session.toggle_like(track),
        Cmd::PlayNext(track) => session.play_next(track),
        Cmd::AddTo(playlist, track) => session.add_to_playlist(playlist, track),
        Cmd::Sync => session.start_sync(),
        Cmd::Search(query) => session.search(&query, SearchKind::Songs),
        Cmd::SaveClient(id, secret) => {
            session.set_client(&id, &secret);
        }
        Cmd::ImportClient(path) => {
            session.import_client(&path);
        }
        Cmd::SignIn => session.sign_in(),
        Cmd::SignOut => session.sign_out(),
        Cmd::FetchArt(..) | Cmd::Quit => {}
    }
}

/// Applies a snapshot on the UI thread.
fn push(ui: &slint::Weak<MainWindow>, snap: Snapshot) {
    let _ = ui.upgrade_in_event_loop(move |ui| {
        ui.set_loading(snap.view.loading);
        ui.set_playing(snap.view.state == PlayState::Playing);
        ui.set_volume(snap.view.volume);
        ui.set_shuffle(snap.view.shuffle);
        // The UI's repeat-mode: 0 off, 1 all, 2 one.
        ui.set_repeat_mode(match snap.view.repeat {
            Repeat::Off => 0,
            Repeat::All => 1,
            Repeat::One => 2,
        });
        ui.set_syncing(snap.view.syncing);
        ui.set_searching(snap.view.searching);
        ui.set_signed_in(snap.view.signed_in);
        ui.set_signing_in(snap.view.signing_in);
        ui.set_client_id(snap.view.client_id.as_str().into());
        if snap.changes.account && snap.view.signed_in {
            ui.set_account_open(false);
        }
        ui.set_memory_text(snap.view.memory.as_str().into());
        let (text, error) = snap.view.status.unwrap_or_default();
        ui.set_status_text(text.into());
        ui.set_status_error(error);
        match &snap.view.duration {
            Some(d) if !d.is_zero() => {
                ui.set_progress((snap.view.position.as_secs_f64() / d.as_secs_f64()) as f32);
                ui.set_position_text(
                    format!("{} / {}", fmt_time(snap.view.position), fmt_time(*d)).into(),
                );
            }
            _ => {
                ui.set_progress(0.0);
                ui.set_position_text(if snap.view.loading {
                    "loading…".into()
                } else {
                    "".into()
                });
            }
        }
        match &snap.view.now {
            Some(t) => {
                ui.set_now_title(SharedString::from(&*t.title));
                ui.set_now_artist(SharedString::from(&*t.artist));
                ui.set_now_initial(initial(&t.title));
                ui.set_now_hue(hue(&t.artist));
            }
            None => {
                ui.set_now_title("".into());
                ui.set_now_artist("".into());
                ui.set_expanded(false);
            }
        }

        VIEW.with(|cell| {
            let Some(view) = cell.borrow().clone() else {
                return;
            };
            let mut v = view.borrow_mut();
            let applied = v.library.borrow_mut().apply(
                &snap.changes,
                SessionData {
                    search: snap.search.as_ref(),
                    ..Default::default()
                },
            );
            match applied {
                Ok(true) => v.sync_list(&ui),
                // Counts may have changed (a like elsewhere).
                Ok(false) if snap.changes.playlist.is_some() => v.sync_playlists(),
                Ok(false) => {}
                Err(err) => tracing::warn!(%err, "reloading the library view"),
            }
            if let Some(id) = &snap.changes.liked {
                v.tracks.rows_changed(id);
                v.queue.rows_changed(id);
            }
            if let Some(queue) = snap.queue {
                v.queue.set_queue(queue);
            }
            let now_id = snap.view.now.as_ref().map(|t| t.video_id.clone());
            let changed = now_id != v.now.as_ref().map(|t| t.video_id.clone());
            if changed || snap.changes.playlist.is_some() {
                let liked = now_id
                    .as_ref()
                    .is_some_and(|id| v.library.borrow().is_liked(id));
                ui.set_liked(liked);
            }
            v.tracks.set_playing(now_id.clone());
            v.queue.set_playing(now_id);
            v.now = snap.view.now;
            if changed {
                v.show_now_art(&ui);
            }
        });
    });
}

thread_local! {
    /// The UI-thread view, for handlers queued from the core thread.
    static VIEW: RefCell<Option<Rc<RefCell<View>>>> = const { RefCell::new(None) };
}

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
    api::models::{LIKED_PLAYLIST_ID, Playlist, Track},
    app::filter_indices,
    audio::{player::PlayState, queue::Repeat},
    session::{Changes, Deps, Session},
    storage::Library,
    ui::fmt_time,
};
use art::ArtSize;
use ui::{MainWindow, PlaylistRow, TrackRow};

/// Decoded thumbnails kept in memory (~36 KB each at 96x96 RGBA).
const THUMB_CACHE: usize = 120;
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
    FetchArt(Arc<str>, ArtSize),
    /// Save state (selected playlist id) and stop.
    Quit(Option<String>),
}

/// Player state pushed to the UI. Small and cheap to clone.
struct Snapshot {
    now: Option<Track>,
    loading: bool,
    state: PlayState,
    position: Duration,
    duration: Option<Duration>,
    volume: f32,
    shuffle: bool,
    repeat: Repeat,
    status: Option<(String, bool)>,
    syncing: bool,
    memory: String,
    changes: Changes,
    /// Current track followed by what's next; only when it changed.
    queue: Option<Arc<[Track]>>,
}

impl Snapshot {
    fn of(session: &Session, changes: Changes, queue: Option<Arc<[Track]>>) -> Self {
        let p = &session.player_status;
        Self {
            now: session.queue.current().cloned(),
            loading: session.loading,
            state: p.state,
            position: p.position,
            duration: p.duration,
            volume: p.volume,
            shuffle: session.queue.shuffle,
            repeat: session.queue.repeat,
            status: session
                .status
                .as_ref()
                .map(|s| (s.text.clone(), s.is_error)),
            syncing: session.syncing,
            memory: session.memory.clone(),
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

/// Track list backed by a shared `Arc<[Track]>`: rows (and their art
/// requests) are built only when the virtualized ListView asks for them.
struct TracksModel {
    tracks: RefCell<Arc<[Track]>>,
    visible: RefCell<Vec<u32>>,
    playing: RefCell<Option<Arc<str>>>,
    art: Rc<RefCell<ArtCache>>,
    notify: ModelNotify,
}

impl TracksModel {
    fn new(art: Rc<RefCell<ArtCache>>) -> Self {
        Self {
            tracks: RefCell::new(Arc::from([])),
            visible: RefCell::default(),
            playing: RefCell::default(),
            art,
            notify: ModelNotify::default(),
        }
    }

    fn set(&self, tracks: Arc<[Track]>, filter: &str) {
        filter_indices(&tracks, filter, &mut self.visible.borrow_mut());
        *self.tracks.borrow_mut() = tracks;
        self.notify.reset();
    }

    fn refilter(&self, filter: &str) {
        filter_indices(
            &self.tracks.borrow(),
            filter,
            &mut self.visible.borrow_mut(),
        );
        self.notify.reset();
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

    /// Re-renders rows showing `id` (art arrived, playing changed).
    fn rows_changed(&self, id: &str) {
        let tracks = self.tracks.borrow();
        let rows: Vec<usize> = self
            .visible
            .borrow()
            .iter()
            .enumerate()
            .filter(|(_, i)| tracks.get(**i as usize).is_some_and(|t| &*t.video_id == id))
            .map(|(row, _)| row)
            .collect();
        drop(tracks);
        for row in rows {
            self.notify.row_changed(row);
        }
    }

    /// Track at a visible row.
    fn track(&self, row: usize) -> Option<Track> {
        let index = *self.visible.borrow().get(row)? as usize;
        self.tracks.borrow().get(index).cloned()
    }

    /// Index into the full list for a visible row.
    fn index(&self, row: usize) -> Option<usize> {
        self.visible.borrow().get(row).map(|&i| i as usize)
    }
}

impl Model for TracksModel {
    type Data = TrackRow;

    fn row_count(&self) -> usize {
        self.visible.borrow().len()
    }

    fn row_data(&self, row: usize) -> Option<TrackRow> {
        let index = *self.visible.borrow().get(row)? as usize;
        let t = self.tracks.borrow().get(index)?.clone();
        let playing = self.playing.borrow().as_ref() == Some(&t.video_id);
        let art = self.art.borrow_mut().thumb(&t.video_id);
        Some(TrackRow {
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

/// UI-thread state.
struct View {
    library: Arc<Library>,
    playlists: Vec<Playlist>,
    selected_playlist: Option<usize>,
    filter: String,
    tracks: Rc<TracksModel>,
    queue: Rc<TracksModel>,
    playlist_rows: Rc<VecModel<PlaylistRow>>,
    art: Rc<RefCell<ArtCache>>,
    /// The playing track (from the last snapshot).
    now: Option<Track>,
}

impl View {
    fn reload_playlists(&mut self, ui: &MainWindow, prefer: Option<&str>) {
        let keep = prefer.map(str::to_owned).or_else(|| self.selected_id());
        self.refresh_playlist_rows();
        let index = keep
            .and_then(|id| self.playlists.iter().position(|p| p.id == id))
            .or((!self.playlists.is_empty()).then_some(0));
        self.select_playlist(ui, index);
    }

    fn refresh_playlist_rows(&mut self) {
        self.playlists = self.library.playlists().unwrap_or_default();
        self.playlist_rows.set_vec(
            self.playlists
                .iter()
                .map(|p| PlaylistRow {
                    title: p.title.as_str().into(),
                    count: p.item_count as i32,
                })
                .collect::<Vec<_>>(),
        );
    }

    fn select_playlist(&mut self, ui: &MainWindow, index: Option<usize>) {
        self.selected_playlist = index;
        ui.set_selected_playlist(index.map_or(-1, |i| i as i32));
        ui.set_selected_track(-1);
        let Some(p) = index.and_then(|i| self.playlists.get(i)) else {
            self.tracks.set(Arc::from([]), "");
            ui.set_tracks_title("".into());
            ui.set_track_count(0);
            return;
        };
        let tracks = self.library.tracks(&p.id).unwrap_or_else(|_| Arc::from([]));
        ui.set_tracks_title(p.title.as_str().into());
        ui.set_track_count(tracks.len() as i32);
        self.tracks.set(tracks, &self.filter);
    }

    fn selected_id(&self) -> Option<String> {
        self.selected_playlist
            .and_then(|i| self.playlists.get(i))
            .map(|p| p.id.clone())
    }

    fn selected_title(&self) -> String {
        self.selected_playlist
            .and_then(|i| self.playlists.get(i))
            .map(|p| p.title.clone())
            .unwrap_or_default()
    }

    /// Selected row's track, else the playing track.
    fn target(&self, ui: &MainWindow, now: Option<Track>) -> Option<Track> {
        usize::try_from(ui.get_selected_track())
            .ok()
            .and_then(|row| self.tracks.track(row))
            .or(now)
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
    let (cmd_tx, cmd_rx) = unbounded_channel();

    let art = Rc::new(RefCell::new(ArtCache {
        thumbs: HashMap::new(),
        order: VecDeque::new(),
        requested: HashSet::new(),
        large: None,
        tx: cmd_tx.clone(),
    }));
    let view = Rc::new(RefCell::new(View {
        library: library.clone(),
        playlists: Vec::new(),
        selected_playlist: None,
        filter: String::new(),
        tracks: Rc::new(TracksModel::new(art.clone())),
        queue: Rc::new(TracksModel::new(art.clone())),
        playlist_rows: Rc::new(VecModel::default()),
        art,
        now: None,
    }));
    VIEW.with(|cell| *cell.borrow_mut() = Some(view.clone()));
    {
        let v = view.borrow();
        ui.set_tracks(ModelRc::from(v.tracks.clone()));
        ui.set_queue_rows(ModelRc::from(v.queue.clone()));
        ui.set_playlists(ModelRc::from(v.playlist_rows.clone()));
    }
    let last = library.get_meta("playlist").ok().flatten();
    view.borrow_mut().reload_playlists(&ui, last.as_deref());
    let library_empty = view.borrow().playlists.is_empty();

    wire_callbacks(&ui, &view, &cmd_tx);

    // Core thread: session + background work. Pushes snapshots to the UI.
    let weak = ui.as_weak();
    let core = std::thread::Builder::new()
        .name("core".into())
        .spawn(move || rt.block_on(core_loop(deps, cmd_rx, weak, library_empty)))?;

    ui.run().context("GUI event loop failed")?;

    let selected = view.borrow().selected_id();
    let _ = cmd_tx.send(Cmd::Quit(selected));
    core.join().map_err(|_| anyhow!("core thread panicked"))?
}

fn wire_callbacks(ui: &MainWindow, view: &Rc<RefCell<View>>, cmd_tx: &UnboundedSender<Cmd>) {
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
    let play_from_list =
        |ui: &MainWindow, view: &RefCell<View>, cmd: Cmd, tx: &UnboundedSender<Cmd>| {
            ui.set_queue_source(view.borrow().selected_title().into());
            let _ = tx.send(cmd);
        };

    on!(on_select_playlist, [tx, view, ui], |index| {
        view.borrow_mut()
            .select_playlist(ui, usize::try_from(index).ok());
    });
    on!(on_select_track, [tx, view, ui], |row| ui
        .set_selected_track(row));
    on!(on_play_track, [tx, view, ui], |row| {
        ui.set_selected_track(row);
        let cmd = {
            let v = view.borrow();
            usize::try_from(row)
                .ok()
                .and_then(|r| v.tracks.index(r))
                .map(|index| Cmd::Play(v.tracks.tracks.borrow().clone(), index))
        };
        if let Some(cmd) = cmd {
            play_from_list(ui, view, cmd, tx);
        }
    });
    on!(on_play_all, [tx, view, ui], || {
        let cmd = {
            let v = view.borrow();
            v.tracks
                .index(0)
                .map(|index| Cmd::Play(v.tracks.tracks.borrow().clone(), index))
        };
        if let Some(cmd) = cmd {
            play_from_list(ui, view, cmd, tx);
        }
    });
    on!(on_shuffle_play, [tx, view, ui], || {
        let tracks = view.borrow().tracks.tracks.borrow().clone();
        if !tracks.is_empty() {
            play_from_list(ui, view, Cmd::ShufflePlay(tracks), tx);
        }
    });
    on!(on_jump, [tx, view, ui], |row| {
        // Row 0 is the playing track itself.
        if let Some(n) = usize::try_from(row).ok().and_then(|r| r.checked_sub(1)) {
            send(Cmd::JumpTo(n), tx);
        }
    });
    on!(on_filter_changed, [tx, view, ui], |text| {
        let mut v = view.borrow_mut();
        v.filter = text.to_string();
        v.tracks.refilter(&v.filter);
        ui.set_selected_track(-1);
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
        // The heart reflects the playing track, so it acts on it too.
        let v = view.borrow();
        if let Some(track) = v.now.clone().or_else(|| v.target(ui, None)) {
            send(Cmd::Like(track), tx);
        }
    });
    on!(on_play_next, [tx, view, ui], || {
        let v = view.borrow();
        if let Some(track) = v.target(ui, v.now.clone()) {
            send(Cmd::PlayNext(track), tx);
        }
    });
    on!(on_add_to, [tx, view, ui], |index| {
        let v = view.borrow();
        let playlist = usize::try_from(index)
            .ok()
            .and_then(|i| v.playlists.get(i))
            .filter(|p| p.id != LIKED_PLAYLIST_ID)
            .cloned();
        if let (Some(playlist), Some(track)) = (playlist, v.target(ui, v.now.clone())) {
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
    let mut queue_key = None;
    let queue = queue_update(&session, &mut queue_key);
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
                    Some(Cmd::Quit(selected)) => {
                        session.save_state(selected.as_deref());
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
        let queue = queue_update(&session, &mut queue_key);
        if dirty || queue.is_some() {
            push(&ui, Snapshot::of(&session, changes, queue));
        }
    }
    session.shutdown();
    Ok(())
}

/// What identifies the queue's contents: current id, position, shuffle, len.
type QueueKey = (Option<Arc<str>>, Option<usize>, bool, usize);

/// The "Up next" rows when the queue changed since `key` was taken.
fn queue_update(session: &Session, key: &mut Option<QueueKey>) -> Option<Arc<[Track]>> {
    let q = &session.queue;
    let new_key = (
        q.current().map(|t| t.video_id.clone()),
        q.position(),
        q.shuffle,
        q.len(),
    );
    if key.as_ref() == Some(&new_key) {
        return None;
    }
    *key = Some(new_key);
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
        Cmd::FetchArt(..) | Cmd::Quit(_) => {}
    }
}

/// Applies a snapshot on the UI thread.
fn push(ui: &slint::Weak<MainWindow>, snap: Snapshot) {
    let _ = ui.upgrade_in_event_loop(move |ui| {
        ui.set_loading(snap.loading);
        ui.set_playing(snap.state == PlayState::Playing);
        ui.set_volume(snap.volume);
        ui.set_shuffle(snap.shuffle);
        ui.set_repeat_mode(match snap.repeat {
            Repeat::Off => 0,
            Repeat::All => 1,
            Repeat::One => 2,
        });
        ui.set_syncing(snap.syncing);
        ui.set_memory_text(snap.memory.as_str().into());
        let (text, error) = snap.status.unwrap_or_default();
        ui.set_status_text(text.into());
        ui.set_status_error(error);
        match &snap.duration {
            Some(d) if !d.is_zero() => {
                ui.set_progress((snap.position.as_secs_f64() / d.as_secs_f64()) as f32);
                ui.set_position_text(
                    format!("{} / {}", fmt_time(snap.position), fmt_time(*d)).into(),
                );
            }
            _ => {
                ui.set_progress(0.0);
                ui.set_position_text(if snap.loading {
                    "loading…".into()
                } else {
                    "".into()
                });
            }
        }
        match &snap.now {
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
            if snap.changes.library {
                v.reload_playlists(&ui, None);
            } else if let Some(id) = &snap.changes.playlist {
                let viewing = v.selected_id().as_deref() == Some(id.as_str());
                let index = v.selected_playlist;
                v.refresh_playlist_rows();
                if viewing {
                    v.select_playlist(&ui, index);
                }
            }
            if let Some(queue) = snap.queue {
                v.queue.set(queue, "");
            }
            let now_id = snap.now.as_ref().map(|t| t.video_id.clone());
            let changed = now_id != v.now.as_ref().map(|t| t.video_id.clone());
            if changed || snap.changes.playlist.is_some() {
                let liked = now_id
                    .as_ref()
                    .is_some_and(|id| v.library.contains(LIKED_PLAYLIST_ID, id).unwrap_or(false));
                ui.set_liked(liked);
            }
            v.tracks.set_playing(now_id.clone());
            v.queue.set_playing(now_id);
            v.now = snap.now;
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

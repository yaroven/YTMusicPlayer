//! Desktop frontend (Slint, software renderer).
//!
//! Threads: the Slint event loop owns the main thread (on macOS that is also
//! where media-key callbacks arrive); the [`Session`] runs on a "core" thread
//! with its own tokio runtime. The UI sends [`Cmd`]s; the core pushes a
//! [`Snapshot`] back whenever player state changes. Library reads (playlists,
//! tracks) happen on the UI thread straight from SQLite.

mod ui;

use std::{
    cell::RefCell,
    rc::Rc,
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::{Context, Result, anyhow};
use slint::{ComponentHandle, Model, ModelNotify, ModelRc, ModelTracker, SharedString, VecModel};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use crate::{
    api::models::{LIKED_PLAYLIST_ID, Playlist, Track},
    app::filter_indices,
    audio::{player::PlayState, queue::Repeat},
    session::{Changes, Deps, Session},
    storage::Library,
    ui::fmt_time,
};
use ui::{MainWindow, PlaylistRow, TrackRow};

enum Cmd {
    Play(Arc<[Track]>, usize),
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
}

impl Snapshot {
    fn of(session: &Session, changes: Changes) -> Self {
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
        }
    }
}

/// Track list backed by the shared `Arc<[Track]>`: rows are built only when
/// the (virtualized) ListView asks for them.
#[derive(Default)]
struct TracksModel {
    tracks: RefCell<Arc<[Track]>>,
    visible: RefCell<Vec<u32>>,
    playing: RefCell<Option<Arc<str>>>,
    notify: ModelNotify,
}

impl TracksModel {
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
        if *self.playing.borrow() != id {
            *self.playing.borrow_mut() = id;
            self.notify.reset();
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
        let tracks = self.tracks.borrow();
        let t = tracks.get(index)?;
        let playing = self.playing.borrow().as_ref() == Some(&t.video_id);
        Some(TrackRow {
            num: if playing {
                "▶".into()
            } else {
                (index + 1).to_string().into()
            },
            title: SharedString::from(&*t.title),
            artist: SharedString::from(&*t.artist),
            time: t
                .duration_secs
                .map(|s| fmt_time(Duration::from_secs(s.into())))
                .unwrap_or_default()
                .into(),
            playing,
        })
    }

    fn model_tracker(&self) -> &dyn ModelTracker {
        &self.notify
    }
}

/// UI-thread state.
struct View {
    library: Arc<Library>,
    playlists: Vec<Playlist>,
    selected_playlist: Option<usize>,
    filter: String,
    tracks: Rc<TracksModel>,
    playlist_rows: Rc<VecModel<PlaylistRow>>,
    /// The playing track (from the last snapshot).
    now: Option<Track>,
}

impl View {
    fn reload_playlists(&mut self, ui: &MainWindow, prefer: Option<&str>) {
        let keep = prefer.map(str::to_owned).or_else(|| self.selected_id());
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
        let index = keep
            .and_then(|id| self.playlists.iter().position(|p| p.id == id))
            .or((!self.playlists.is_empty()).then_some(0));
        self.select_playlist(ui, index);
    }

    fn select_playlist(&mut self, ui: &MainWindow, index: Option<usize>) {
        self.selected_playlist = index;
        ui.set_selected_playlist(index.map_or(-1, |i| i as i32));
        ui.set_selected_track(-1);
        let Some(p) = index.and_then(|i| self.playlists.get(i)) else {
            self.tracks.set(Arc::from([]), "");
            ui.set_tracks_title("".into());
            return;
        };
        let tracks = self.library.tracks(&p.id).unwrap_or_else(|_| Arc::from([]));
        ui.set_tracks_title(p.title.as_str().into());
        self.tracks.set(tracks, &self.filter);
    }

    fn selected_id(&self) -> Option<String> {
        self.selected_playlist
            .and_then(|i| self.playlists.get(i))
            .map(|p| p.id.clone())
    }

    /// Selected row's track, else the playing track.
    fn target(&self, ui: &MainWindow, now: Option<Track>) -> Option<Track> {
        usize::try_from(ui.get_selected_track())
            .ok()
            .and_then(|row| self.tracks.track(row))
            .or(now)
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

    let view = Rc::new(RefCell::new(View {
        library: library.clone(),
        playlists: Vec::new(),
        selected_playlist: None,
        filter: String::new(),
        tracks: Rc::new(TracksModel::default()),
        playlist_rows: Rc::new(VecModel::default()),
        now: None,
    }));
    VIEW.with(|cell| *cell.borrow_mut() = Some(view.clone()));
    {
        let v = view.borrow();
        ui.set_tracks(ModelRc::from(v.tracks.clone()));
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

    on!(on_select_playlist, [tx, view, ui], |index| {
        view.borrow_mut()
            .select_playlist(ui, usize::try_from(index).ok());
    });
    on!(on_select_track, [tx, view, ui], |row| ui
        .set_selected_track(row));
    on!(on_play_track, [tx, view, ui], |row| {
        ui.set_selected_track(row);
        let v = view.borrow();
        if let Some(index) = usize::try_from(row).ok().and_then(|r| v.tracks.index(r)) {
            send(Cmd::Play(v.tracks.tracks.borrow().clone(), index), tx);
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
    let mut session = Session::new(deps)?;
    session.startup(library_empty);
    push(&ui, Snapshot::of(&session, Changes::default()));

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
        if dirty {
            push(&ui, Snapshot::of(&session, changes));
        }
    }
    session.shutdown();
    Ok(())
}

fn apply(session: &mut Session, cmd: Cmd) {
    match cmd {
        Cmd::Play(tracks, index) => session.play(tracks, index),
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
        Cmd::Quit(_) => {}
    }
}

/// Applies a snapshot on the UI thread.
fn push(ui: &slint::Weak<MainWindow>, snap: Snapshot) {
    let _ = ui.upgrade_in_event_loop(move |ui| {
        ui.set_loading(snap.loading);
        ui.set_playing(snap.state == PlayState::Playing);
        ui.set_volume(snap.volume);
        ui.set_shuffle(snap.shuffle);
        ui.set_repeat_on(snap.repeat != Repeat::Off);
        ui.set_repeat_label(
            match snap.repeat {
                Repeat::Off | Repeat::All => "Repeat",
                Repeat::One => "Repeat 1",
            }
            .into(),
        );
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
            }
            None => {
                ui.set_now_title("Nothing playing".into());
                ui.set_now_artist("".into());
            }
        }

        // Library-side refreshes need the View, reachable via user data.
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
                v.playlists = v.library.playlists().unwrap_or_default();
                let rows: Vec<PlaylistRow> = v
                    .playlists
                    .iter()
                    .map(|p| PlaylistRow {
                        title: p.title.as_str().into(),
                        count: p.item_count as i32,
                    })
                    .collect();
                v.playlist_rows.set_vec(rows);
                if viewing {
                    v.select_playlist(&ui, index);
                }
            }
            let now_id = snap.now.as_ref().map(|t| t.video_id.clone());
            let old_id = v.now.as_ref().map(|t| t.video_id.clone());
            if now_id != old_id || snap.changes.playlist.is_some() {
                let liked = now_id
                    .as_ref()
                    .is_some_and(|id| v.library.contains(LIKED_PLAYLIST_ID, id).unwrap_or(false));
                ui.set_liked(liked);
            }
            v.tracks.set_playing(now_id);
            v.now = snap.now;
        });
    });
}

thread_local! {
    /// The UI-thread view, for snapshot handlers queued from the core thread.
    static VIEW: RefCell<Option<Rc<RefCell<View>>>> = const { RefCell::new(None) };
}

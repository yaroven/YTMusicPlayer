//! Desktop frontend (Slint, software renderer).
//!
//! Threads: the Slint event loop owns the main thread (on macOS that is also
//! where media-key callbacks arrive); the [`Session`] runs on a "core" thread
//! with its own tokio runtime. The UI sends [`Cmd`]s; the core pushes a
//! [`Snapshot`] back whenever player state changes. Library reads (playlists,
//! tracks) happen on the UI thread straight from SQLite.
//!
//! Album art is fetched lazily for rows the list actually shows and kept in
//! small bounded caches ([`ArtCache`]); only the playing track gets a large
//! cover.

mod art;
mod ui;

use std::{
    cell::RefCell,
    collections::{HashMap, HashSet, VecDeque},
    rc::Rc,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
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
    api::models::{LIKED_PLAYLIST_ID, Playlist, Track},
    audio::{player::PlayState, queue::Repeat},
    catalog::{Item, ItemKind, Page, SearchKind, Shelf},
    config::settings::Settings,
    fmt::fmt_time,
    library_view::{LibraryView, SessionData, Source},
    lyrics::Lyrics,
    session::{Changes, Deps, SearchResults, Session, SessionView, Sleep},
    storage::Library,
};
use art::ArtSize;
use ui::{CardRow, LyricLine, MainWindow, PlaylistRow, ShelfRow, TrackRow};

/// Decoded thumbnails kept in memory (~36 KB each at 96x96 RGBA): two
/// screens of rows plus the queue panel on a tall window.
const THUMB_CACHE: usize = 80;
/// Decoded card images kept beyond those on screen (~64 KB each).
const CARD_CACHE: usize = 48;
/// Rows of the "Up next" list sent to the UI.
const QUEUE_ROWS: usize = 200;
/// Parallel thumbnail downloads.
const ART_FETCHES: usize = 4;
/// Sleep timer choices are minutes; these mean "end of song" / "off".
const SLEEP_END_OF_TRACK: i32 = 0;
const SLEEP_OFF: i32 = -1;

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
    Sync,
    Search(String, SearchKind),
    OpenPage(String),
    LoadHome,
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
    /// The opened page, when `changes.page`.
    page: Option<Arc<Page>>,
    /// The home feed, when `changes.home`.
    home: Option<Arc<[Shelf]>>,
    /// The playing track's lyrics, when `changes.lyrics`.
    lyrics: Option<Arc<Lyrics>>,
    /// Related shelves, when `changes.related`.
    related: Option<Arc<[Shelf]>>,
    /// Current track followed by what's next; only when it changed.
    queue: Option<Arc<[Track]>>,
}

impl Snapshot {
    fn of(session: &Session, changes: Changes, queue: Option<Arc<[Track]>>) -> Self {
        Self {
            view: session.view(),
            search: session.search.clone().filter(|_| changes.search),
            page: session.page.clone().filter(|_| changes.page),
            home: session.home.clone().filter(|_| changes.home),
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
            changes,
            queue,
        }
    }
}

/// The playing track's big cover: id, image, average colour.
type LargeArt = (Arc<str>, Image, (u8, u8, u8));

/// Bounded LRU-ish map of decoded images.
struct Images {
    map: HashMap<Arc<str>, Image>,
    order: VecDeque<Arc<str>>,
    cap: usize,
}

impl Images {
    fn new(cap: usize) -> Self {
        Self {
            map: HashMap::new(),
            order: VecDeque::new(),
            cap,
        }
    }

    fn insert(&mut self, key: Arc<str>, image: Image) {
        if self.map.insert(key.clone(), image).is_none() {
            self.order.push_back(key);
        }
        while self.order.len() > self.cap {
            if let Some(old) = self.order.pop_front() {
                self.map.remove(&old);
            }
        }
    }
}

/// Bounded caches of decoded album art (UI thread only).
struct ArtCache {
    /// By video id.
    thumbs: Images,
    /// By image URL.
    cards: Images,
    /// Requested and not yet answered, or failed: never re-requested.
    requested: HashSet<Arc<str>>,
    /// The single large cover (for the playing track).
    large: Option<LargeArt>,
    tx: UnboundedSender<Cmd>,
}

impl ArtCache {
    /// The image for `key` if decoded, else requests it (once).
    fn get(&mut self, key: &Arc<str>, size: ArtSize) -> Option<Image> {
        let images = match size {
            ArtSize::Card => &self.cards,
            _ => &self.thumbs,
        };
        if let Some(image) = images.map.get(key) {
            return Some(image.clone());
        }
        if self.requested.insert(key.clone()) {
            let _ = self.tx.send(Cmd::FetchArt(key.clone(), size));
        }
        None
    }

    fn thumb(&mut self, id: &Arc<str>) -> Option<Image> {
        self.get(id, ArtSize::Thumb)
    }

    fn insert(&mut self, key: Arc<str>, size: ArtSize, image: Image) {
        self.requested.remove(&key);
        match size {
            ArtSize::Card => self.cards.insert(key, image),
            _ => self.thumbs.insert(key, image),
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

    fn track(&self, row: usize) -> Option<Track> {
        match &self.rows {
            Rows::Library(view) => view.borrow().row(row).map(|(_, t)| t.clone()),
            Rows::Queue(tracks) => tracks.borrow().get(row).cloned(),
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
        let (t, liked, downloaded) = match &self.rows {
            Rows::Library(view) => {
                let view = view.borrow();
                let (_, t) = view.row(row)?;
                (
                    t.clone(),
                    view.is_liked(&t.video_id),
                    view.is_downloaded(&t.video_id),
                )
            }
            // Queue rows show no like button.
            Rows::Queue(tracks) => (tracks.borrow().get(row)?.clone(), false, false),
        };
        let playing = self.playing.borrow().as_ref() == Some(&t.video_id);
        let art = self.art.borrow_mut().thumb(&t.video_id);
        Some(TrackRow {
            liked,
            downloaded,
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

/// A row of cards and the art key of each (to fill in art as it arrives).
struct Cards {
    model: Rc<VecModel<CardRow>>,
    keys: Vec<Option<Arc<str>>>,
}

impl Cards {
    fn new() -> Self {
        Self {
            model: Rc::new(VecModel::default()),
            keys: Vec::new(),
        }
    }

    fn of(items: &[Item], art: &mut ArtCache) -> Self {
        let mut cards = Self::new();
        cards.set(items, art);
        cards
    }

    fn set(&mut self, items: &[Item], art: &mut ArtCache) {
        self.keys = items.iter().map(|i| i.thumbnail.clone()).collect();
        self.fill(items, art);
    }

    fn fill(&self, items: &[Item], art: &mut ArtCache) {
        let rows: Vec<CardRow> = items
            .iter()
            .map(|item| {
                let image = item
                    .thumbnail
                    .as_ref()
                    .and_then(|url| art.get(url, ArtSize::Card));
                CardRow {
                    title: SharedString::from(&*item.title),
                    subtitle: SharedString::from(&*item.subtitle),
                    initial: initial(&item.title),
                    hue: hue(&item.title),
                    has_art: image.is_some(),
                    art: image.unwrap_or_default(),
                    round: item.kind == ItemKind::Artist,
                    saved: false,
                }
            })
            .collect();
        self.model.set_vec(rows);
    }

    fn art_arrived(&self, key: &Arc<str>, image: &Image) {
        for (row, k) in self.keys.iter().enumerate() {
            if k.as_ref() == Some(key)
                && let Some(mut card) = self.model.row_data(row)
            {
                card.art = image.clone();
                card.has_art = true;
                self.model.set_row_data(row, card);
            }
        }
    }
}

/// Titled rows of cards.
struct Shelves {
    model: Rc<VecModel<ShelfRow>>,
    rows: Vec<Cards>,
}

impl Shelves {
    fn new() -> Self {
        Self {
            model: Rc::new(VecModel::default()),
            rows: Vec::new(),
        }
    }

    fn set(&mut self, shelves: &[Shelf], art: &mut ArtCache) {
        self.rows = shelves.iter().map(|s| Cards::of(&s.items, art)).collect();
        let rows: Vec<ShelfRow> = shelves
            .iter()
            .zip(&self.rows)
            .map(|(s, cards)| ShelfRow {
                title: s.title.as_str().into(),
                cards: ModelRc::from(cards.model.clone()),
            })
            .collect();
        self.model.set_vec(rows);
    }

    fn art_arrived(&self, key: &Arc<str>, image: &Image) {
        for cards in &self.rows {
            cards.art_arrived(key, image);
        }
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

/// Lyrics of the playing track and the line being sung.
struct LyricsView {
    lyrics: Option<Arc<Lyrics>>,
    model: Rc<VecModel<LyricLine>>,
    current: Option<usize>,
}

impl LyricsView {
    fn set(&mut self, ui: &MainWindow, lyrics: Option<Arc<Lyrics>>) {
        let synced = lyrics.as_ref().is_some_and(|l| l.synced());
        let rows: Vec<LyricLine> = lyrics
            .iter()
            .flat_map(|l| &l.lines)
            .map(|line| LyricLine {
                text: line.text.as_str().into(),
                state: if synced { 0 } else { 3 },
            })
            .collect();
        self.model.set_vec(rows);
        ui.set_lyrics_source(
            lyrics
                .as_ref()
                .map(|l| l.source.as_str())
                .unwrap_or_default()
                .into(),
        );
        self.lyrics = lyrics;
        self.current = None;
        ui.set_lyrics_current(-1);
    }

    /// Highlights the line sung at `position`.
    fn follow(&mut self, ui: &MainWindow, position: Duration) {
        let Some(lyrics) = &self.lyrics else {
            return;
        };
        let current = lyrics.current_line(position);
        if current == self.current {
            return;
        }
        let (a, b) = (
            self.current.unwrap_or(0).min(current.unwrap_or(0)),
            self.current.unwrap_or(0).max(current.unwrap_or(0)),
        );
        for row in a..=b.min(self.model.row_count().saturating_sub(1)) {
            if let Some(mut line) = self.model.row_data(row) {
                line.state = match current {
                    Some(c) if row == c => 1,
                    Some(c) if row < c => 2,
                    _ => 0,
                };
                self.model.set_row_data(row, line);
            }
        }
        self.current = current;
        ui.set_lyrics_current(current.map_or(-1, |c| c as i32));
    }
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
    cards: Cards,
    shelves: Shelves,
    related: Shelves,
    related_data: Arc<[Shelf]>,
    lyrics: LyricsView,
    /// The home feed (last received).
    home: Option<Arc<[Shelf]>>,
    /// Art URL of the page header.
    header_art: Option<Arc<str>>,
    /// Track for "New playlist…" from the save-to menu.
    pending_add: Option<Track>,
    art: Rc<RefCell<ArtCache>>,
    /// The playing track (from the last snapshot).
    now: Option<Track>,
    notifications: bool,
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
    fn sync_list(&mut self, ui: &MainWindow) {
        self.sync_playlists();
        let lib = self.library.borrow();
        let source = lib.source().clone();
        ui.set_nav(match source {
            Source::Home => 1,
            Source::History => 2,
            Source::Downloads => 3,
            Source::Saved(ItemKind::Artist) => 5,
            Source::Saved(_) => 4,
            Source::Search => 6,
            _ => 0,
        });
        ui.set_list_mode(
            if !lib.items().is_empty() || matches!(source, Source::Saved(_)) {
                1
            } else if !lib.shelves().is_empty() {
                2
            } else {
                0
            },
        );
        ui.set_selected_playlist(lib.selected_playlist().map_or(-1, |i| i as i32));
        ui.set_search_query(lib.search_query().unwrap_or_default().into());
        ui.set_search_kind(match lib.search_kind() {
            Some(SearchKind::Albums) => 1,
            Some(SearchKind::Artists) => 2,
            Some(SearchKind::Playlists) => 3,
            _ => 0,
        });
        ui.set_tracks_title(lib.title().into());
        ui.set_tracks_subtitle(lib.subtitle().into());
        ui.set_can_back(lib.can_go_back());
        ui.set_selected_track(lib.selected_row().map_or(-1, |r| r as i32));
        ui.set_page_kind(match lib.page_kind() {
            Some(ItemKind::Album) => 1,
            Some(ItemKind::Artist) => 2,
            Some(ItemKind::Playlist) => 3,
            _ => 0,
        });
        ui.set_page_art_round(lib.page_kind() == Some(ItemKind::Artist));
        ui.set_own_playlist(
            lib.shown_playlist()
                .is_some_and(|p| p.id != LIKED_PLAYLIST_ID),
        );
        if lib.filter().is_empty() {
            // A new list starts unfiltered: empty the search field too.
            ui.set_clear_search(ui.get_clear_search().wrapping_add(1));
        }
        let (items, shelves) = (lib.items().clone(), lib.shelves().clone());
        let header: Option<Arc<str>> = lib.thumbnail().map(Arc::from);
        drop(lib);
        self.sync_saved(ui);
        {
            let art = self.art.clone();
            let mut art = art.borrow_mut();
            self.cards.set(&items, &mut art);
            self.shelves.set(&shelves, &mut art);
        }
        self.header_art = header.clone();
        let image = header.and_then(|url| self.art.borrow_mut().get(&url, ArtSize::Card));
        ui.set_has_page_art(image.is_some());
        ui.set_page_art(image.unwrap_or_default());
        self.tracks.reset();
    }

    /// Whether the shown page is saved / followed.
    fn sync_saved(&self, ui: &MainWindow) {
        let lib = self.library.borrow();
        ui.set_page_saved(
            lib.page_item()
                .is_some_and(|(item, _)| lib.is_saved(&item.id)),
        );
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

    /// The track of list row `row`, or the playing one for -1.
    fn row_track(&self, row: i32) -> Option<Track> {
        match usize::try_from(row) {
            Ok(r) => self.tracks.track(r),
            Err(_) => self.now.clone(),
        }
    }

    /// What clicking a card does: songs play with their radio, the rest
    /// open their page.
    fn open_item(item: &Item) -> Option<Cmd> {
        match (&item.track, item.kind) {
            (Some(track), _) => Some(Cmd::Radio(track.clone())),
            (None, ItemKind::Song) => None,
            (None, _) => Some(Cmd::OpenPage(item.id.to_string())),
        }
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

    /// Art for `key` arrived on the UI thread.
    fn art_arrived(&mut self, ui: &MainWindow, key: Arc<str>, size: ArtSize, art: art::Art) {
        let image = Image::from_rgba8(art.pixels);
        let is_now = self.now.as_ref().is_some_and(|t| t.video_id == key);
        match size {
            ArtSize::Thumb => {
                self.art
                    .borrow_mut()
                    .insert(key.clone(), size, image.clone());
                self.tracks.rows_changed(&key);
                self.queue.rows_changed(&key);
            }
            ArtSize::Card => {
                self.art
                    .borrow_mut()
                    .insert(key.clone(), size, image.clone());
                self.cards.art_arrived(&key, &image);
                self.shelves.art_arrived(&key, &image);
                self.related.art_arrived(&key, &image);
                if self.header_art.as_ref() == Some(&key) {
                    ui.set_page_art(image);
                    ui.set_has_page_art(true);
                }
                return;
            }
            ArtSize::Large if is_now => {
                self.art.borrow_mut().large = Some((key.clone(), image, art.tint));
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
pub fn run(rt: tokio::runtime::Runtime, deps: Deps, settings: &Settings) -> Result<()> {
    let ui = MainWindow::new().context("cannot open a window (no display?)")?;
    *WINDOW.lock().unwrap_or_else(|e| e.into_inner()) = Some(ui.as_weak());
    let library = deps.library.clone();
    let library_view = Rc::new(RefCell::new(LibraryView::new(library.clone())?));
    let (cmd_tx, cmd_rx) = unbounded_channel();

    let art = Rc::new(RefCell::new(ArtCache {
        thumbs: Images::new(THUMB_CACHE),
        cards: Images::new(CARD_CACHE),
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
        cards: Cards::new(),
        shelves: Shelves::new(),
        related: Shelves::new(),
        related_data: Arc::from([]),
        lyrics: LyricsView {
            lyrics: None,
            model: Rc::new(VecModel::default()),
            current: None,
        },
        home: None,
        header_art: None,
        pending_add: None,
        art,
        now: None,
        notifications: settings.notifications,
    }));
    VIEW.with(|cell| *cell.borrow_mut() = Some(view.clone()));
    {
        let mut v = view.borrow_mut();
        ui.set_tracks(ModelRc::from(v.tracks.clone()));
        ui.set_queue_rows(ModelRc::from(v.queue.clone()));
        ui.set_playlists(ModelRc::from(v.playlist_rows.clone()));
        ui.set_add_choices(ModelRc::from(v.add_rows.clone()));
        ui.set_cards(ModelRc::from(v.cards.model.clone()));
        ui.set_shelves(ModelRc::from(v.shelves.model.clone()));
        ui.set_related(ModelRc::from(v.related.model.clone()));
        ui.set_lyrics(ModelRc::from(v.lyrics.model.clone()));
        v.sync_list(&ui);
    }
    ui.set_autoplay(deps.autoplay);
    ui.set_set_normalize(settings.normalize_volume);
    ui.set_set_notifications(settings.notifications);
    ui.set_set_tray(settings.tray);
    ui.set_set_crossfade(settings.crossfade);
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
    // Runs a navigation step on the library view, then mirrors it.
    let navigate = |ui: &MainWindow,
                    view: &RefCell<View>,
                    step: &dyn Fn(&mut LibraryView, &View) -> Result<()>| {
        let mut v = view.borrow_mut();
        let result = {
            let lib = v.library.clone();
            let mut lib = lib.borrow_mut();
            step(&mut lib, &v)
        };
        if let Err(err) = result {
            tracing::warn!(%err, "loading the list");
            ui.set_status_text(format!("library: {err:#}").into());
            ui.set_status_error(true);
        }
        v.sync_list(ui);
    };

    on!(on_select_playlist, [tx, view, ui], |index| {
        if let Ok(i) = usize::try_from(index) {
            navigate(ui, view, &|lib, _| lib.select_playlist(Some(i)));
        }
    });
    on!(on_nav_to, [tx, view, ui], |nav| {
        if nav == 1 {
            send(Cmd::LoadHome, tx);
        }
        navigate(ui, view, &|lib, v| match nav {
            1 => lib.show_home(v.home.as_deref()),
            2 => lib.show_history(),
            3 => lib.show_downloads(),
            4 => lib.show_saved(ItemKind::Album),
            5 => lib.show_saved(ItemKind::Artist),
            _ => {
                lib.show_results();
                Ok(())
            }
        });
    });
    on!(on_go_back, [tx, view, ui], || navigate(
        ui,
        view,
        &|lib, _| {
            lib.back();
            Ok(())
        }
    ));
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
    on!(on_start_radio, [tx, view, ui], || {
        // The selected song's radio, else the list's first song's.
        let v = view.borrow();
        v.selection_from(ui);
        let lib = v.library.borrow();
        let seed = lib
            .selected_track()
            .cloned()
            .or_else(|| lib.row(0).map(|(_, t)| t.clone()));
        if let Some(track) = seed {
            ui.set_queue_source(format!("{} radio", track.title).into());
            send(Cmd::Radio(track), tx);
        }
    });
    on!(on_jump, [tx, view, ui], |row| {
        // Row 0 is the playing track itself.
        if let Some(n) = usize::try_from(row).ok().and_then(|r| r.checked_sub(1)) {
            send(Cmd::JumpTo(n), tx);
        }
    });
    on!(on_filter_changed, [tx, view, ui], |text| {
        let v = view.borrow();
        v.library.borrow_mut().set_filter(text.as_str());
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
    on!(on_dislike_now, [tx, view, ui], || {
        if let Some(track) = view.borrow().now.clone() {
            send(Cmd::Dislike(track), tx);
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
            // A new query keeps the category being looked at.
            let kind = {
                let v = view.borrow();
                let lib = v.library.borrow();
                match lib.source() {
                    Source::Search => lib.search_kind().unwrap_or(SearchKind::Songs),
                    _ => SearchKind::Songs,
                }
            };
            send(Cmd::Search(query.to_owned(), kind), tx);
        }
    });
    on!(on_search_category, [tx, view, ui], |k| {
        let query = view
            .borrow()
            .library
            .borrow()
            .search_query()
            .map(str::to_owned);
        let kind = usize::try_from(k)
            .ok()
            .and_then(|k| SearchKind::ALL.get(k).copied());
        if let (Some(query), Some(kind)) = (query, kind) {
            send(Cmd::Search(query, kind), tx);
        }
    });
    on!(on_like_row, [tx, view, ui], |row| {
        if let Some(track) = usize::try_from(row)
            .ok()
            .and_then(|r| view.borrow().tracks.track(r))
        {
            send(Cmd::Like(track), tx);
        }
    });
    on!(on_row_action, [tx, view, ui], |row, action| {
        let mut v = view.borrow_mut();
        let Some(track) = v.row_track(row) else {
            return;
        };
        let cmd = match action.as_str() {
            "next" => Cmd::PlayNext(track),
            "queue" => Cmd::AddToQueue(vec![track]),
            "radio" => {
                ui.set_queue_source(format!("{} radio", track.title).into());
                Cmd::Radio(track)
            }
            "dislike" => Cmd::Dislike(track),
            "download" if v.library.borrow().is_downloaded(&track.video_id) => {
                Cmd::RemoveDownload(track.video_id)
            }
            "download" => Cmd::Download(vec![track]),
            "remove" => {
                let lib = v.library.borrow();
                match (lib.source(), lib.shown_playlist()) {
                    (Source::Downloads, _) => Cmd::RemoveDownload(track.video_id),
                    (_, Some(p)) if row >= 0 => Cmd::RemoveFrom(p.clone(), track),
                    _ => return,
                }
            }
            "add" => {
                // The save-to popup acts on the selection (or the playing track).
                ui.set_selected_track(row.max(-1));
                v.selection_from(ui);
                v.pending_add = None;
                return;
            }
            _ => return,
        };
        send(cmd, tx);
    });
    on!(on_open_card, [tx, view, ui], |i| {
        let v = view.borrow();
        let lib = v.library.borrow();
        if let Some(cmd) = usize::try_from(i)
            .ok()
            .and_then(|i| lib.items().get(i))
            .and_then(View::open_item)
        {
            send(cmd, tx);
        }
    });
    on!(on_open_shelf_card, [tx, view, ui], |s, c| {
        let v = view.borrow();
        let lib = v.library.borrow();
        let item = usize::try_from(s)
            .ok()
            .zip(usize::try_from(c).ok())
            .and_then(|(s, c)| lib.shelves().get(s)?.items.get(c));
        if let Some(cmd) = item.and_then(View::open_item) {
            send(cmd, tx);
        }
    });
    on!(on_open_related, [tx, view, ui], |s, c| {
        let v = view.borrow();
        let item = usize::try_from(s)
            .ok()
            .zip(usize::try_from(c).ok())
            .and_then(|(s, c)| v.related_data.get(s)?.items.get(c));
        if let Some(cmd) = item.and_then(View::open_item) {
            if matches!(cmd, Cmd::OpenPage(_)) {
                ui.set_expanded(false);
            }
            send(cmd, tx);
        }
    });
    on!(on_toggle_saved, [tx, view, ui], || {
        let item = view.borrow().library.borrow().page_item();
        if let Some((item, channel)) = item {
            send(Cmd::ToggleSaved(item, channel), tx);
        }
    });
    on!(on_download_all, [tx, view, ui], || {
        let tracks = view.borrow().library.borrow().all_tracks();
        if let Some(tracks) = tracks {
            send(Cmd::Download(tracks.to_vec()), tx);
        }
    });
    on!(on_new_playlist, [tx, view, ui], |name| {
        let name = name.trim();
        if !name.is_empty() {
            let track = view.borrow_mut().pending_add.take();
            send(Cmd::CreatePlaylist(name.to_owned(), track), tx);
        }
    });
    on!(on_rename_playlist, [tx, view, ui], |name| {
        let name = name.trim();
        let playlist = view.borrow().library.borrow().shown_playlist().cloned();
        if let (false, Some(p)) = (name.is_empty(), playlist) {
            send(Cmd::RenamePlaylist(p, name.to_owned()), tx);
        }
    });
    on!(on_delete_playlist, [tx, view, ui], || {
        let playlist = view.borrow().library.borrow().shown_playlist().cloned();
        if let Some(p) = playlist {
            send(Cmd::DeletePlaylist(p), tx);
        }
    });
    on!(on_queue_action, [tx, view, ui], |row, action| {
        // Row 0 is the playing track; upcoming tracks count from row 1.
        let Some(n) = usize::try_from(row).ok().and_then(|r| r.checked_sub(1)) else {
            return;
        };
        let cmd = match action.as_str() {
            "up" if n > 0 => Cmd::QueueMove(n, n - 1),
            "down" => Cmd::QueueMove(n, n + 1),
            "remove" => Cmd::QueueRemove(n),
            _ => return,
        };
        send(cmd, tx);
    });
    on!(on_clear_queue, [tx, view, ui], || send(Cmd::ClearQueue, tx));
    on!(on_toggle_autoplay, [tx, view, ui], || send(
        Cmd::ToggleAutoplay,
        tx
    ));
    on!(on_set_sleep, [tx, view, ui], |minutes| send(
        Cmd::Sleep(minutes),
        tx
    ));
    on!(on_now_tab_changed, [tx, view, ui], |tab| send(
        Cmd::Tabs(tab == 1, tab == 2),
        tx
    ));
    on!(on_setting_toggled, [tx, view, ui], |name| {
        match name.as_str() {
            "normalize_volume" => {
                let on = !ui.get_set_normalize();
                ui.set_set_normalize(on);
                send(Cmd::Normalize(on), tx);
            }
            "notifications" => {
                let on = !ui.get_set_notifications();
                ui.set_set_notifications(on);
                view.borrow_mut().notifications = on;
                send(Cmd::StoreSetting("notifications", on), tx);
            }
            "tray" => {
                let on = !ui.get_set_tray();
                ui.set_set_tray(on);
                send(Cmd::StoreSetting("tray", on), tx);
            }
            _ => {}
        }
    });
    on!(on_crossfade_changed, [tx, view, ui], |secs| {
        // The slider reports every move: act on whole seconds only.
        let secs = secs.round().clamp(0.0, 12.0);
        if secs != ui.get_set_crossfade() {
            ui.set_set_crossfade(secs);
            send(Cmd::Crossfade(secs as u64), tx);
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
        let mut v = view.borrow_mut();
        match index {
            // "New playlist…" from the save-to menu: create, then add.
            -1 => v.pending_add = v.target(ui),
            // The sidebar's "+": an empty playlist.
            -2 => v.pending_add = None,
            i => {
                let playlist = usize::try_from(i)
                    .ok()
                    .and_then(|i| v.library.borrow().add_choice(i).cloned());
                if let (Some(playlist), Some(track)) = (playlist, v.target(ui)) {
                    send(Cmd::AddTo(playlist, track), tx);
                }
            }
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
        Cmd::Sync => session.start_sync(),
        Cmd::Search(query, kind) => session.search(&query, kind),
        Cmd::OpenPage(id) => session.open_page(&id),
        Cmd::LoadHome => session.load_home(false),
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

/// "23 min" / "end of song" for the sleep timer.
fn sleep_text(sleep: Option<Sleep>) -> String {
    match sleep {
        None => String::new(),
        Some(Sleep::EndOfTrack) => "end of song".into(),
        Some(Sleep::At(at)) => {
            let left = at.saturating_duration_since(Instant::now()).as_secs();
            format!("{} min", left.div_ceil(60))
        }
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
        ui.set_autoplay(snap.view.autoplay);
        ui.set_sleep_text(sleep_text(snap.view.sleep).into());
        ui.set_syncing(snap.view.syncing);
        ui.set_searching(snap.view.searching);
        ui.set_signed_in(snap.view.signed_in);
        ui.set_signing_in(snap.view.signing_in);
        ui.set_client_id(snap.view.client_id.as_str().into());
        if snap.changes.account && snap.view.signed_in {
            ui.set_account_open(false);
        }
        ui.set_memory_text(snap.view.memory.as_str().into());
        let (text, error) = snap.view.status.clone().unwrap_or_default();
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
            if let Some(home) = &snap.home {
                v.home = Some(home.clone());
            }
            let applied = v.library.borrow_mut().apply(
                &snap.changes,
                SessionData {
                    search: snap.search.as_ref(),
                    page: snap.page.as_deref(),
                    home: v.home.as_deref(),
                },
            );
            match applied {
                Ok(true) => v.sync_list(&ui),
                // Counts may have changed (a like elsewhere).
                Ok(false) if snap.changes.playlist.is_some() => v.sync_playlists(),
                Ok(false) => {}
                Err(err) => tracing::warn!(%err, "reloading the library view"),
            }
            if snap.changes.saved {
                v.sync_saved(&ui);
            }
            if snap.changes.downloads {
                v.tracks.reset();
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
            if changed {
                // What's shown about the old track goes; the new arrives later.
                v.lyrics.set(&ui, None);
                ui.set_lyrics_loading(true);
                v.related_data = Arc::from([]);
                let art = v.art.clone();
                v.related.set(&[], &mut art.borrow_mut());
            }
            if snap.changes.lyrics {
                v.lyrics.set(&ui, snap.lyrics.clone());
                ui.set_lyrics_loading(false);
            }
            v.lyrics.follow(&ui, snap.view.position);
            if let Some(related) = &snap.related {
                v.related_data = related.clone();
                let art = v.art.clone();
                v.related.set(related, &mut art.borrow_mut());
            }
            if changed || snap.changes.playlist.is_some() || snap.changes.liked.is_some() {
                let liked = now_id
                    .as_ref()
                    .is_some_and(|id| v.library.borrow().is_liked(id));
                ui.set_liked(liked);
            }
            v.tracks.set_playing(now_id.clone());
            v.queue.set_playing(now_id);
            v.now = snap.view.now.clone();
            if changed {
                v.show_now_art(&ui);
                if v.notifications
                    && let Some(track) = &v.now
                {
                    crate::notify::track_changed(track);
                }
            }
        });
    });
}

thread_local! {
    /// The UI-thread view, for handlers queued from the core thread.
    static VIEW: RefCell<Option<Rc<RefCell<View>>>> = const { RefCell::new(None) };
}

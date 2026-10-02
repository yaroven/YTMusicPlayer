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
mod tray;
mod ui;

use std::{
    cell::RefCell,
    collections::{HashMap, HashSet, VecDeque},
    rc::Rc,
    sync::Arc,
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
/// Songs listed above an album / artist page's shelves.
const PAGE_SONGS: usize = 5;
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
    /// Device names, when `changes.cast`.
    cast_devices: Option<Vec<String>>,
    /// What installing an update did, and the release page.
    update_done: Option<(crate::update::Installed, String)>,
    cast_scanning: bool,
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

    fn clear(&mut self) {
        self.map = HashMap::new();
        self.order = VecDeque::new();
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
            ArtSize::Card | ArtSize::Round => &self.cards,
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
            ArtSize::Card | ArtSize::Round => self.cards.insert(key, image),
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
    /// Show only the first rows (pages list a few songs above shelves).
    limit: std::cell::Cell<Option<usize>>,
    playing: RefCell<Option<Arc<str>>>,
    art: Rc<RefCell<ArtCache>>,
    notify: ModelNotify,
}

impl TracksModel {
    fn new(rows: Rows, art: Rc<RefCell<ArtCache>>) -> Self {
        Self {
            rows,
            limit: std::cell::Cell::new(None),
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
        let all = match &self.rows {
            Rows::Library(view) => view.borrow().len(),
            Rows::Queue(tracks) => tracks.borrow().len(),
        };
        self.limit.get().map_or(all, |limit| all.min(limit))
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
                    .and_then(|url| art.get(url, card_size(item.kind)));
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

/// "chrome", or "cookies.txt" for a file path.
fn cookies_label(source: &str) -> &str {
    if source.contains(['/', '\\']) {
        "cookies.txt"
    } else {
        source
    }
}

/// Artists get round pictures.
fn card_size(kind: ItemKind) -> ArtSize {
    match kind {
        ItemKind::Artist => ArtSize::Round,
        _ => ArtSize::Card,
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
    /// Asked for and not answered yet.
    loading: bool,
}

impl LyricsView {
    fn set(&mut self, lyrics: Option<Arc<Lyrics>>) {
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
        self.lyrics = lyrics;
        self.current = None;
    }

    /// Highlights the line sung at `position`.
    fn follow(&mut self, position: Duration) {
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
    }

    fn render(&self, ui: &MainWindow) {
        ui.set_lyrics_source(
            self.lyrics
                .as_ref()
                .map(|l| l.source.as_str())
                .unwrap_or_default()
                .into(),
        );
        ui.set_lyrics_current(self.current.map_or(-1, |c| c as i32));
        ui.set_lyrics_loading(self.loading);
    }
}

/// Settings the window shows and changes.
struct Prefs {
    cookies: String,
    normalize: bool,
    notifications: bool,
    tray: bool,
    crossfade: f32,
}

/// UI-thread state; outlives the window, which exists only while it's
/// open (closed to the tray it's destroyed with its pixel buffers). What
/// the list shows is the [`LibraryView`]; this mirrors it into the
/// window's properties and models.
struct View {
    tx: UnboundedSender<Cmd>,
    store: Arc<Library>,
    /// The last snapshot's player state (for a window opened later).
    last: Option<SessionView>,
    queue_source: RefCell<SharedString>,
    cast_names: Rc<VecModel<SharedString>>,
    cast_scanning: bool,
    prefs: Prefs,
    /// Size and position of the closed window, to reopen it the same.
    geometry: Option<(slint::PhysicalSize, slint::PhysicalPosition)>,
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
        let list_mode = if !lib.items().is_empty() || matches!(source, Source::Saved(_)) {
            1
        } else if !lib.shelves().is_empty() {
            2
        } else {
            0
        };
        ui.set_list_mode(list_mode);
        // Pages with shelves aren't virtualized: a few songs, then "Show all".
        let limited = list_mode == 2 && matches!(source, Source::Page(_));
        self.tracks.limit.set(limited.then_some(PAGE_SONGS));
        ui.set_page_songs(if limited { lib.len() as i32 } else { 0 });
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
        let page_kind = lib.page_kind();
        drop(lib);
        self.sync_saved(ui);
        {
            let art = self.art.clone();
            let mut art = art.borrow_mut();
            self.cards.set(&items, &mut art);
            self.shelves.set(&shelves, &mut art);
        }
        self.header_art = header.clone();
        let size = card_size(page_kind.unwrap_or(ItemKind::Album));
        let image = header.and_then(|url| self.art.borrow_mut().get(&url, size));
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
    fn art_arrived(
        &mut self,
        ui: Option<&MainWindow>,
        key: Arc<str>,
        size: ArtSize,
        art: art::Art,
    ) {
        let Some(ui) = ui else {
            // The window closed meanwhile: nothing shows it.
            self.art.borrow_mut().requested.remove(&key);
            return;
        };
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
            ArtSize::Card | ArtSize::Round => {
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

    /// Fills a new window with everything it shows.
    fn attach(&mut self, ui: &MainWindow) {
        ui.set_tracks(ModelRc::from(self.tracks.clone()));
        ui.set_queue_rows(ModelRc::from(self.queue.clone()));
        ui.set_playlists(ModelRc::from(self.playlist_rows.clone()));
        ui.set_add_choices(ModelRc::from(self.add_rows.clone()));
        ui.set_cards(ModelRc::from(self.cards.model.clone()));
        ui.set_shelves(ModelRc::from(self.shelves.model.clone()));
        ui.set_related(ModelRc::from(self.related.model.clone()));
        ui.set_lyrics(ModelRc::from(self.lyrics.model.clone()));
        ui.set_cast_devices(ModelRc::from(self.cast_names.clone()));
        ui.set_set_normalize(self.prefs.normalize);
        ui.set_set_notifications(self.prefs.notifications);
        ui.set_set_tray(self.prefs.tray);
        ui.set_set_crossfade(self.prefs.crossfade);
        ui.set_set_cookies(cookies_label(&self.prefs.cookies).into());
        ui.set_queue_source(self.queue_source.borrow().clone());
        ui.set_app_version(env!("CARGO_PKG_VERSION").into());
        if let Some(width) = self
            .store
            .get_meta("sidebar_width")
            .ok()
            .flatten()
            .and_then(|w| w.parse::<f32>().ok())
        {
            ui.set_sidebar_width(width);
        }
        self.sync_list(ui);
        self.render_player(ui);
        self.lyrics.render(ui);
        let (art, related) = (self.art.clone(), self.related_data.clone());
        self.related.set(&related, &mut art.borrow_mut());
        self.render_liked(ui);
        self.show_now_art(ui);
    }

    /// The window closed: drop what only it needed (art, cards).
    fn detach(&mut self) {
        {
            let mut art = self.art.borrow_mut();
            art.thumbs.clear();
            art.cards.clear();
            art.requested.clear();
            art.large = None;
        }
        let art = self.art.clone();
        let mut art = art.borrow_mut();
        self.cards.set(&[], &mut art);
        self.shelves.set(&[], &mut art);
        self.related.set(&[], &mut art);
        // Lyrics / Related aren't on screen any more.
        let _ = self.tx.send(Cmd::Tabs(false, false));
    }

    /// Player bar, status line and account state from the last snapshot.
    fn render_player(&self, ui: &MainWindow) {
        let Some(view) = &self.last else {
            return;
        };
        ui.set_loading(view.loading);
        ui.set_playing(view.state == PlayState::Playing);
        ui.set_volume(view.volume);
        ui.set_shuffle(view.shuffle);
        // The UI's repeat-mode: 0 off, 1 all, 2 one.
        ui.set_repeat_mode(match view.repeat {
            Repeat::Off => 0,
            Repeat::All => 1,
            Repeat::One => 2,
        });
        ui.set_autoplay(view.autoplay);
        ui.set_sleep_text(sleep_text(view.sleep).into());
        ui.set_casting(view.casting.clone().unwrap_or_default().into());
        ui.set_cast_scanning(self.cast_scanning);
        ui.set_update_version(view.update.clone().unwrap_or_default().into());
        ui.set_updating(view.updating);
        ui.set_syncing(view.syncing);
        ui.set_searching(view.searching);
        ui.set_signed_in(view.signed_in);
        ui.set_signing_in(view.signing_in);
        ui.set_client_id(view.client_id.as_str().into());
        ui.set_memory_text(view.memory.as_str().into());
        let (text, error) = view.status.clone().unwrap_or_default();
        ui.set_status_text(text.into());
        ui.set_status_error(error);
        match &view.duration {
            Some(d) if !d.is_zero() => {
                ui.set_progress((view.position.as_secs_f64() / d.as_secs_f64()) as f32);
                let on = view
                    .casting
                    .as_ref()
                    .map(|name| format!(" · {name}"))
                    .unwrap_or_default();
                ui.set_position_text(
                    format!("{} / {}{on}", fmt_time(view.position), fmt_time(*d)).into(),
                );
            }
            _ => {
                ui.set_progress(0.0);
                ui.set_position_text(if view.loading {
                    "loading…".into()
                } else {
                    "".into()
                });
            }
        }
        match &view.now {
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
    }

    fn render_liked(&self, ui: &MainWindow) {
        let liked = self
            .now
            .as_ref()
            .is_some_and(|t| self.library.borrow().is_liked(&t.video_id));
        ui.set_liked(liked);
    }

    /// Takes in a snapshot from the core; renders it when a window is open.
    fn absorb(&mut self, snap: Snapshot, ui: Option<&MainWindow>) {
        if let Some((done, page)) = &snap.update_done {
            use crate::update::Installed;
            match done {
                Installed::Replaced => {
                    // The new binary starts once this process is gone.
                    match crate::update::relaunch_gui() {
                        Ok(()) => {
                            let _ = slint::quit_event_loop();
                        }
                        Err(err) => tracing::warn!(%err, "restarting after the update"),
                    }
                }
                Installed::InstallerStarted { quit: true } => {
                    let _ = slint::quit_event_loop();
                }
                Installed::InstallerStarted { quit: false } => {}
                Installed::Manual(_) => {
                    let _ = open::that_detached(page);
                }
            }
        }
        if let Some(home) = &snap.home {
            self.home = Some(home.clone());
        }
        let applied = self.library.borrow_mut().apply(
            &snap.changes,
            SessionData {
                search: snap.search.as_ref(),
                page: snap.page.as_deref(),
                home: self.home.as_deref(),
            },
        );
        let replaced = applied.unwrap_or_else(|err| {
            tracing::warn!(%err, "reloading the library view");
            false
        });
        if let Some(queue) = snap.queue {
            self.queue.set_queue(queue);
        }
        if let Some(devices) = snap.cast_devices {
            let names: Vec<SharedString> = devices.into_iter().map(SharedString::from).collect();
            self.cast_names.set_vec(names);
        }
        self.cast_scanning = snap.cast_scanning;
        if let Some(id) = &snap.changes.liked {
            self.tracks.rows_changed(id);
            self.queue.rows_changed(id);
        }
        if snap.changes.downloads {
            self.tracks.reset();
        }
        if replaced || snap.changes.playlist.is_some() {
            self.sync_playlists();
        }

        let now_id = snap.view.now.as_ref().map(|t| t.video_id.clone());
        let changed = now_id != self.now.as_ref().map(|t| t.video_id.clone());
        if changed {
            // What's shown about the old track goes; the new arrives later.
            self.lyrics.set(None);
            self.lyrics.loading = true;
            self.related_data = Arc::from([]);
        }
        if snap.changes.lyrics {
            self.lyrics.set(snap.lyrics.clone());
            self.lyrics.loading = false;
        }
        self.lyrics.follow(snap.view.position);
        if let Some(related) = &snap.related {
            self.related_data = related.clone();
        }
        self.tracks.set_playing(now_id.clone());
        self.queue.set_playing(now_id);
        self.now = snap.view.now.clone();
        if changed {
            let tip = self.now.as_ref().map_or_else(
                || "ytm-player".to_owned(),
                |t| format!("{} — {}", t.title, t.artist),
            );
            TRAY.with(|cell| {
                if let Some(tray) = cell.borrow().as_ref() {
                    tray.set_tooltip(&tip);
                }
            });
            if self.prefs.notifications
                && let Some(track) = &self.now
            {
                crate::notify::track_changed(track);
            }
        }
        let signed_in = snap.view.signed_in;
        self.last = Some(snap.view);

        let Some(ui) = ui else {
            return;
        };
        self.render_player(ui);
        if snap.changes.account && signed_in {
            ui.set_account_open(false);
        }
        if replaced {
            self.sync_list(ui);
        }
        if snap.changes.saved {
            self.sync_saved(ui);
        }
        self.lyrics.render(ui);
        if changed || snap.related.is_some() {
            let (art, related) = (self.art.clone(), self.related_data.clone());
            self.related.set(&related, &mut art.borrow_mut());
        }
        if changed || snap.changes.playlist.is_some() || snap.changes.liked.is_some() {
            self.render_liked(ui);
        }
        if changed {
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

/// Opens the window again, or brings it to the front (called from the
/// tray and the single-instance thread).
pub fn raise() {
    let _ = slint::invoke_from_event_loop(|| {
        if let Some(ui) = current_ui() {
            use slint::winit_030::WinitWindowAccessor;
            let window = ui.window();
            window.set_minimized(false);
            let _ = window.show();
            window.with_winit_window(|w| w.focus_window());
        } else {
            #[cfg(target_os = "macos")]
            in_dock(true);
            if let Err(err) = open_window() {
                tracing::warn!("reopening the window: {err:#}");
            }
        }
    });
}

fn current_ui() -> Option<MainWindow> {
    UI.with(|cell| cell.borrow().as_ref().map(|ui| ui.clone_strong()))
}

/// Creates the window from the current state and shows it.
fn open_window() -> Result<MainWindow> {
    let view = VIEW
        .with(|cell| cell.borrow().clone())
        .context("GUI state missing")?;
    let ui = MainWindow::new().context("cannot open a window (no display?)")?;
    let (tx, store) = {
        let mut v = view.borrow_mut();
        v.attach(&ui);
        (v.tx.clone(), v.store.clone())
    };
    wire_callbacks(&ui, &view, &tx, &store);
    ui.window().on_close_requested(|| {
        let tray = TRAY.with(|cell| cell.borrow().as_ref().is_some_and(tray::Tray::available));
        let keep = VIEW.with(|cell| {
            cell.borrow()
                .as_ref()
                .is_some_and(|v| v.borrow().prefs.tray)
        });
        if !(tray && keep) {
            let _ = slint::quit_event_loop();
        } else if cfg!(not(windows)) {
            // Media keys on Windows hang on to this window: only hide it there.
            let _ = slint::invoke_from_event_loop(close_window);
        }
        slint::CloseRequestResponse::HideWindow
    });
    if let Some((size, position)) = view.borrow().geometry {
        ui.window().set_size(size);
        ui.window().set_position(position);
    }
    ui.show().context("cannot show the window")?;
    UI.with(|cell| *cell.borrow_mut() = Some(ui.clone_strong()));
    Ok(ui)
}

/// Destroys the window (the music plays on; the tray brings it back).
fn close_window() {
    let Some(ui) = UI.with(|cell| cell.borrow_mut().take()) else {
        return;
    };
    VIEW.with(|cell| {
        if let Some(view) = cell.borrow().as_ref() {
            let mut v = view.borrow_mut();
            v.geometry = Some((ui.window().size(), ui.window().position()));
            v.detach();
        }
    });
    let _ = ui.hide();
    #[cfg(target_os = "macos")]
    in_dock(false);
}

/// Shows or hides the app in the Dock and the app switcher (macOS): a
/// window closed to the tray leaves only the menu bar icon, like other
/// menu bar players.
#[cfg(target_os = "macos")]
fn in_dock(shown: bool) {
    use objc2::{class, msg_send, runtime::AnyObject};
    // NSApplicationActivationPolicyRegular = 0, Accessory = 1.
    let policy: isize = if shown { 0 } else { 1 };
    // SAFETY: main thread (Slint's event loop); NSApplication's shared
    // instance exists once the event loop runs.
    unsafe {
        let app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
        if app.is_null() {
            return;
        }
        let _: bool = msg_send![app, setActivationPolicy: policy];
        if shown {
            let _: () = msg_send![app, activateIgnoringOtherApps: true];
        }
    }
}

/// Runs the GUI until it quits. `rt` runs the core on its own thread.
pub fn run(rt: tokio::runtime::Runtime, deps: Deps, settings: &Settings) -> Result<()> {
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
        tx: cmd_tx.clone(),
        store: library.clone(),
        last: None,
        queue_source: RefCell::default(),
        cast_names: Rc::new(VecModel::default()),
        cast_scanning: false,
        prefs: Prefs {
            cookies: settings.cookies_from_browser.trim().to_lowercase(),
            normalize: settings.normalize_volume,
            notifications: settings.notifications,
            tray: settings.tray,
            crossfade: settings.crossfade,
        },
        geometry: None,
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
            loading: false,
        },
        home: None,
        header_art: None,
        pending_add: None,
        art,
        now: None,
    }));
    VIEW.with(|cell| *cell.borrow_mut() = Some(view.clone()));
    let library_empty = view.borrow().library.borrow().playlists().is_empty();

    let ui = open_window()?;
    ui.set_autoplay(deps.autoplay);
    // First start: nothing works without an account, so ask right away.
    if !deps.account.signed_in() {
        ui.set_download_file(downloaded_client_name().into());
        ui.set_account_open(true);
    }
    #[cfg(windows)]
    if let Some(hwnd) = window_handle(&ui) {
        // Media keys (SMTC) attach to the window; the session creates them.
        crate::media::set_window_handle(hwnd);
    }
    drop(ui);

    if settings.tray {
        let (rt, tx) = (rt.handle().clone(), cmd_tx.clone());
        // macOS wants the status item created inside the running event loop.
        slint::Timer::single_shot(Duration::ZERO, move || {
            let handler = move |action| match action {
                tray::Action::TogglePause => {
                    let _ = tx.send(Cmd::TogglePause);
                }
                tray::Action::Next => {
                    let _ = tx.send(Cmd::Next);
                }
                tray::Action::Prev => {
                    let _ = tx.send(Cmd::Prev);
                }
                tray::Action::Show => raise(),
                tray::Action::Quit => {
                    let _ = slint::quit_event_loop();
                }
            };
            match tray::Tray::new(&rt, handler) {
                Ok(t) => TRAY.with(|cell| *cell.borrow_mut() = Some(t)),
                Err(err) => tracing::warn!("tray icon: {err:#}"),
            }
        });
    }

    // Freed frame buffers would otherwise stay in our footprint (Linux).
    let trim = slint::Timer::default();
    trim.start(
        slint::TimerMode::Repeated,
        Duration::from_secs(3),
        crate::sysmem::release_free_memory,
    );

    // Core thread: session + background work. Pushes snapshots to the UI.
    let core = std::thread::Builder::new()
        .name("core".into())
        .spawn(move || rt.block_on(core_loop(deps, cmd_rx, library_empty)))?;

    slint::run_event_loop_until_quit().context("GUI event loop failed")?;
    UI.with(|cell| cell.borrow_mut().take());
    TRAY.with(|cell| cell.borrow_mut().take());

    view.borrow().library.borrow().save();
    let _ = cmd_tx.send(Cmd::Quit);
    core.join().map_err(|_| anyhow!("core thread panicked"))?
}

/// The window's HWND.
#[cfg(windows)]
fn window_handle(ui: &MainWindow) -> Option<usize> {
    use slint::winit_030::{
        WinitWindowAccessor,
        winit::raw_window_handle::{HasWindowHandle, RawWindowHandle},
    };
    ui.window()
        .with_winit_window(|w| match w.window_handle().ok()?.as_raw() {
            RawWindowHandle::Win32(h) => Some(h.hwnd.get() as usize),
            _ => None,
        })
        .flatten()
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
                let source: SharedString = view.borrow().library.borrow().source_name().into();
                ui.set_queue_source(source.clone());
                *view.borrow().queue_source.borrow_mut() = source;
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
            let source: SharedString = format!("{} radio", track.title).into();
            ui.set_queue_source(source.clone());
            *v.queue_source.borrow_mut() = source;
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
                let source: SharedString = format!("{} radio", track.title).into();
                ui.set_queue_source(source.clone());
                *v.queue_source.borrow_mut() = source;
                Cmd::Radio(track)
            }
            "dislike" => Cmd::Dislike(track),
            "artist" => Cmd::OpenArtist(track),
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
        if action.as_str() == "artist" {
            let track = usize::try_from(row)
                .ok()
                .and_then(|r| view.borrow().queue.track(r));
            if let Some(track) = track {
                ui.set_expanded(false);
                send(Cmd::OpenArtist(track), tx);
            }
            return;
        }
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
    on!(on_cast_scan, [tx, view, ui], || send(Cmd::CastScan, tx));
    on!(on_cast_to, [tx, view, ui], |i| send(
        Cmd::CastTo(usize::try_from(i).ok()),
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
                view.borrow_mut().prefs.normalize = on;
                send(Cmd::Normalize(on), tx);
            }
            "notifications" => {
                let on = !ui.get_set_notifications();
                ui.set_set_notifications(on);
                view.borrow_mut().prefs.notifications = on;
                send(Cmd::StoreSetting("notifications", on), tx);
            }
            "tray" => {
                let on = !ui.get_set_tray();
                ui.set_set_tray(on);
                view.borrow_mut().prefs.tray = on;
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
            view.borrow_mut().prefs.crossfade = secs;
            send(Cmd::Crossfade(secs as u64), tx);
        }
    });
    on!(on_update_now, [tx, view, ui], || send(
        Cmd::InstallUpdate,
        tx
    ));
    on!(on_check_updates, [tx, view, ui], || send(
        Cmd::CheckUpdates,
        tx
    ));
    on!(on_show_all_songs, [tx, view, ui], || navigate(
        ui,
        view,
        &|lib, _| {
            lib.show_songs();
            Ok(())
        }
    ));
    on!(on_cookies_cycle, [tx, view, ui], || {
        // Off, then the browsers that have a profile on this computer.
        let choices: Vec<&str> = std::iter::once("")
            .chain(crate::audio::extractor::installed_browsers())
            .collect();
        let mut v = view.borrow_mut();
        let at = choices.iter().position(|c| *c == v.prefs.cookies);
        let next = choices[at.map_or(0, |i| (i + 1) % choices.len())];
        v.prefs.cookies = next.to_owned();
        ui.set_set_cookies(cookies_label(next).into());
        send(Cmd::CookiesBrowser(next.to_owned()), tx);
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
    library_empty: bool,
) -> Result<()> {
    let http = deps.http.clone();
    let fetches = Arc::new(Semaphore::new(ART_FETCHES));
    let mut session = Session::new(deps)?;
    session.startup(library_empty);
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
                    Some(Cmd::FetchArt(id, size)) => {
                        dirty = false;
                        fetch_art(&http, &fetches, id, size);
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
fn fetch_art(http: &reqwest::Client, permits: &Arc<Semaphore>, id: Arc<str>, size: ArtSize) {
    let (http, permits) = (http.clone(), permits.clone());
    tokio::spawn(async move {
        let _permit = permits.acquire_owned().await;
        match art::fetch(&http, &id, size).await {
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

/// Applies a snapshot on the UI thread (to the window, if one is open).
fn push(snap: Snapshot) {
    let _ = slint::invoke_from_event_loop(move || {
        let ui = current_ui();
        VIEW.with(|cell| {
            if let Some(view) = cell.borrow().clone() {
                view.borrow_mut().absorb(snap, ui.as_ref());
            }
        });
    });
}

thread_local! {
    /// The tray icon, when enabled (UI thread).
    static TRAY: RefCell<Option<tray::Tray>> = const { RefCell::new(None) };
    /// The open window, if any.
    static UI: RefCell<Option<MainWindow>> = const { RefCell::new(None) };
    /// The UI-thread view, for handlers queued from the core thread.
    static VIEW: RefCell<Option<Rc<RefCell<View>>>> = const { RefCell::new(None) };
}

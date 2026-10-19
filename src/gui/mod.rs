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
#[cfg(target_os = "macos")]
mod macos;
mod tray;
mod ui;
#[cfg(any(target_os = "macos", windows))]
mod video;

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
    i18n::Lang,
    library_view::{LibraryView, SessionData, Source},
    lyrics::Lyrics,
    session::{Changes, Deps, SearchResults, Session, SessionView, Sleep},
    storage::Library,
    tr, trf,
};
use art::ArtSize;
use ui::{CardRow, L, LyricLine, MainWindow, PlaylistRow, ShelfRow, TrackRow};

/// Decoded thumbnails kept in memory (~27 KB each at 96x96 RGB): a screen
/// of rows plus the queue panel on a tall window.
const THUMB_CACHE: usize = 56;
/// Decoded card images kept beyond those on screen (~77 KB each).
const CARD_CACHE: usize = 28;
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
    LoadHome,
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
    /// Explore shelves, when `changes.explore`.
    explore: Option<Arc<[Shelf]>>,
    /// Search suggestions, when `changes.suggestions`.
    suggestions: Option<Arc<[String]>>,
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
            explore: session.explore.clone().filter(|_| changes.explore),
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

/// The playing track's big cover: id, image, average colour.
/// Video id, cover, average colour, blurred backdrop.
type LargeArt = (Arc<str>, Image, (u8, u8, u8), Image);

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
        let (t, liked, downloaded, plays) = match &self.rows {
            Rows::Library(view) => {
                let view = view.borrow();
                let (_, t) = view.row(row)?;
                // Sorted by plays: the count shows next to the artist.
                let plays = (view.sort() == crate::library_view::SortBy::MostPlayed)
                    .then(|| view.plays(&t.video_id));
                (
                    t.clone(),
                    view.is_liked(&t.video_id),
                    view.is_downloaded(&t.video_id),
                    plays,
                )
            }
            // Queue rows show no like button.
            Rows::Queue(tracks) => (tracks.borrow().get(row)?.clone(), false, false, None),
        };
        let playing = self.playing.borrow().as_ref() == Some(&t.video_id);
        let art = self.art.borrow_mut().thumb(&t.video_id);
        Some(TrackRow {
            liked,
            downloaded,
            explicit: t.explicit,
            initial: initial(&t.title),
            hue: hue(&t.artist),
            title: SharedString::from(&*t.title),
            artist: match plays {
                Some(n) if n > 0 => format!("{} · ▶ {n}", t.artist).into(),
                _ => SharedString::from(&*t.artist),
            },
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
                    explicit: item.explicit,
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

/// Suggestions the dropdown shows.
const SUGGESTIONS_SHOWN: usize = 8;

/// How long typing pauses before suggestions are asked for.
const SUGGEST_DELAY: Duration = Duration::from_millis(250);

thread_local! {
    /// Debounces search suggestions (see `on_filter_changed`).
    static SUGGEST_TIMER: slint::Timer = slint::Timer::default();
    /// The row last played by a click, and when (see `on_play_track`).
    static LAST_PLAY: std::cell::Cell<Option<(i32, std::time::Instant)>> = const { std::cell::Cell::new(None) };
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

/// Lyrics of the playing track, karaoke style: the sung line lights up
/// word by word (see [`Lyrics::progress`]), instrumental breaks show three
/// dots. A timer redraws only when a word starts, not every frame.
struct LyricsView {
    lyrics: Option<Arc<Lyrics>>,
    model: Rc<VecModel<LyricLine>>,
    /// Model row of lyrics line 0: 1 when an intro row comes first.
    offset: usize,
    /// Model row being sung and its words (or dots) lit.
    progress: Option<(usize, usize)>,
    /// Asked for and not answered yet.
    loading: bool,
    /// Position at the last snapshot, and when it came (`None`: paused).
    clock: (Duration, Option<std::time::Instant>),
    ticker: slint::Timer,
}

/// How often the timer looks for the next word (no redraw unless one starts).
const LYRICS_TICK: Duration = Duration::from_millis(60);

impl LyricsView {
    fn new() -> Self {
        Self {
            lyrics: None,
            model: Rc::new(VecModel::default()),
            offset: 0,
            progress: None,
            loading: false,
            clock: (Duration::ZERO, None),
            ticker: slint::Timer::default(),
        }
    }

    fn set(&mut self, lyrics: Option<Arc<Lyrics>>) {
        let synced = lyrics.as_ref().is_some_and(|l| l.synced());
        let intro = lyrics
            .as_ref()
            .filter(|_| synced)
            .and_then(|l| l.lines.first()?.at)
            .is_some_and(|at| at >= crate::lyrics::INTRO_GAP);
        let mut rows: Vec<LyricLine> = Vec::new();
        if intro {
            rows.push(LyricLine {
                instrumental: true,
                at: 0,
                ..LyricLine::default()
            });
        }
        rows.extend(lyrics.iter().flat_map(|l| &l.lines).map(|line| LyricLine {
            text: line.text.as_str().into(),
            state: if synced { 0 } else { 3 },
            at: line.at.map_or(-1, |at| at.as_millis() as i32),
            instrumental: synced && line.instrumental(),
            ..LyricLine::default()
        }));
        self.model.set_vec(rows);
        self.offset = usize::from(intro);
        self.lyrics = lyrics;
        self.progress = None;
    }

    /// The player's position at a snapshot; the timer runs on from it.
    fn follow(&mut self, position: Duration, playing: bool) {
        self.clock = (position, playing.then(std::time::Instant::now));
        self.tick();
    }

    fn position(&self) -> Duration {
        let (at, since) = self.clock;
        since.map_or(at, |t| at + t.elapsed())
    }

    /// Lights the words sung by now; true when something changed.
    fn tick(&mut self) -> bool {
        let Some(lyrics) = self.lyrics.clone().filter(|l| l.synced()) else {
            return false;
        };
        let position = self.position();
        let first = lyrics.lines.first().and_then(|l| l.at).unwrap_or_default();
        let progress = if self.offset == 1 && position < first {
            // The intro: dots light up until the first line.
            let lit = (position.as_secs_f64() / first.as_secs_f64().max(0.1) * 4.0) as usize;
            Some((0, lit.min(3)))
        } else {
            lyrics
                .progress(position)
                .map(|p| (p.line + self.offset, p.chars))
        };
        if progress == self.progress {
            return false;
        }
        let old = self.progress.map(|(row, _)| row);
        let new = progress.map(|(row, _)| row);
        if old != new {
            // States between the old and the new line change (seeks too).
            let (a, b) = (
                old.unwrap_or(0).min(new.unwrap_or(0)),
                old.unwrap_or(0).max(new.unwrap_or(0)),
            );
            for row in a..=b.min(self.model.row_count().saturating_sub(1)) {
                if let Some(mut line) = self.model.row_data(row) {
                    line.state = match new {
                        Some(c) if row == c => 1,
                        Some(c) if row < c => 2,
                        _ => 0,
                    };
                    if line.state != 1 {
                        line.dots = 0;
                    }
                    self.model.set_row_data(row, line);
                }
            }
        }
        if let Some((row, lit)) = progress
            && let Some(mut line) = self.model.row_data(row)
        {
            if line.instrumental {
                line.dots = lit as i32;
            } else {
                let text = row
                    .checked_sub(self.offset)
                    .and_then(|i| lyrics.lines.get(i))
                    .map(|l| l.text.as_str())
                    .unwrap_or_default();
                line.styled = karaoke_markup(text, lit);
            }
            self.model.set_row_data(row, line);
        }
        self.progress = progress;
        true
    }

    /// Runs the word timer while synced lyrics play in an open window.
    fn schedule(&self, view: std::rc::Weak<RefCell<View>>) {
        let synced = self.lyrics.as_ref().is_some_and(|l| l.synced());
        let wanted = synced && self.clock.1.is_some() && current_ui().is_some();
        if wanted == self.ticker.running() {
            return;
        }
        if !wanted {
            self.ticker.stop();
            return;
        }
        self.ticker
            .start(slint::TimerMode::Repeated, LYRICS_TICK, move || {
                if let Some(view) = view.upgrade()
                    && let Ok(mut view) = view.try_borrow_mut()
                {
                    view.lyrics.tick();
                }
            });
    }

    fn render(&self, ui: &MainWindow) {
        ui.set_lyrics_source(
            self.lyrics
                .as_ref()
                .map(|l| l.source.as_str())
                .unwrap_or_default()
                .into(),
        );
        ui.set_lyrics_current(self.progress.map_or(-1, |(row, _)| row as i32));
        ui.set_lyrics_loading(self.loading);
    }
}

/// The sung line: its first `lit` characters bright, the rest dimmed
/// (StyledText markup; the text is escaped).
fn karaoke_markup(text: &str, lit: usize) -> slint::StyledText {
    fn escape(text: &str) -> String {
        let mut out = String::with_capacity(text.len() + 8);
        for c in text.chars() {
            if c.is_ascii_punctuation() {
                out.push('\\');
            }
            out.push(c);
        }
        out
    }
    let split = text
        .char_indices()
        .nth(lit)
        .map_or(text.len(), |(at, _)| at);
    let (done, rest) = text.split_at(split);
    // Markdown drops spaces at the edges of the spans: keep one between
    // them as plain text.
    let space = if done.ends_with(' ') || rest.starts_with(' ') {
        " "
    } else {
        ""
    };
    let (done, rest) = (escape(done.trim_end()), escape(rest.trim()));
    let markup = format!(
        "**<font color=\"#ffffff\">{done}</font>{space}<font color=\"#ffffff66\">{rest}</font>**"
    );
    slint::StyledText::from_markdown(&markup)
        .unwrap_or_else(|_| slint::StyledText::from_plain_text(text))
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
    /// Explore shelves (last received).
    explore: Option<Arc<[Shelf]>>,
    /// What YouTube Music suggests for the query being typed.
    suggestion_rows: Rc<VecModel<SharedString>>,
    /// The clip playing over the cover, when the video is on.
    #[cfg(any(target_os = "macos", windows))]
    video: Option<video::VideoView>,
    /// The language the window speaks.
    lang: Lang,
    /// Where the video goes (window coordinates, logical pixels).
    #[cfg(any(target_os = "macos", windows))]
    video_rect: (f32, f32, f32, f32),
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
        let lang = self.lang;
        let row = |p: &Playlist| PlaylistRow {
            title: crate::library_view::playlist_title(lang, p).into(),
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
        ui.set_sort_label(match lib.sort() {
            crate::library_view::SortBy::Default => SharedString::new(),
            sort => sort.label(self.lang).into(),
        });
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
            Some((large_id, image, tint, backdrop)) if *large_id == id => {
                ui.set_now_art(image.clone());
                ui.set_now_backdrop(backdrop.clone());
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
        let image = art.pixels.image();
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
                let backdrop = art.backdrop.map(Image::from_rgb8).unwrap_or_default();
                self.art.borrow_mut().large = Some((key.clone(), image, art.tint, backdrop));
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
        ui.set_search_suggestions(ModelRc::from(self.suggestion_rows.clone()));
        ui.set_cast_devices(ModelRc::from(self.cast_names.clone()));
        ui.set_set_normalize(self.prefs.normalize);
        ui.set_set_notifications(self.prefs.notifications);
        ui.set_set_tray(self.prefs.tray);
        ui.set_set_crossfade(self.prefs.crossfade);
        ui.set_set_cookies(cookies_label(&self.prefs.cookies).into());
        ui.set_yt_window(cfg!(any(target_os = "macos", windows)));
        ui.set_queue_source(self.queue_source.borrow().clone());
        ui.set_app_version(env!("CARGO_PKG_VERSION").into());
        apply_language(ui, self.lang);
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
        self.lyrics.ticker.stop();
        // The web view belongs to the window that is going away.
        #[cfg(any(target_os = "macos", windows))]
        if let Some(video) = self.video.take() {
            let at = video.position();
            let _ = self.tx.send(Cmd::VideoOff(at));
        }
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
        ui.set_sleep_text(sleep_text(view.sleep, view.lang).into());
        ui.set_casting(view.casting.clone().unwrap_or_default().into());
        ui.set_cast_scanning(self.cast_scanning);
        ui.set_update_version(view.update.clone().unwrap_or_default().into());
        ui.set_updating(view.updating);
        ui.set_syncing(view.syncing);
        ui.set_searching(view.searching);
        ui.set_downloading(view.downloading as i32);
        ui.set_offline(view.offline);
        ui.set_watching(view.watching);
        ui.set_signed_in(view.signed_in);
        ui.set_signing_in(view.signing_in);
        ui.set_yt_signed_in(view.youtube_signed_in);
        ui.set_yt_signing_in(view.youtube_signing_in);
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
        // The core decides the language (settings, or the system's).
        if snap.view.lang != self.lang {
            self.lang = snap.view.lang;
            self.library.borrow_mut().set_language(self.lang);
            if let Some(ui) = ui {
                apply_language(ui, self.lang);
                self.sync_list(ui);
            }
        }
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
        if let Some(explore) = &snap.explore {
            self.explore = Some(explore.clone());
        }
        if let Some(suggestions) = &snap.suggestions {
            self.suggestion_rows.set_vec(
                suggestions
                    .iter()
                    .take(SUGGESTIONS_SHOWN)
                    .map(|s| SharedString::from(s.as_str()))
                    .collect::<Vec<_>>(),
            );
        }
        let applied = self.library.borrow_mut().apply(
            &snap.changes,
            SessionData {
                search: snap.search.as_ref(),
                page: snap.page.as_deref(),
                home: self.home.as_deref(),
                explore: self.explore.as_deref(),
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
        self.lyrics
            .follow(snap.view.position, snap.view.state == PlayState::Playing);
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
        let signed_in = snap.view.signed_in || snap.view.youtube_signed_in;
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
        self.lyrics.schedule(view_weak());
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

/// Puts the chosen language's labels into the window.
fn apply_language(ui: &MainWindow, lang: Lang) {
    let t = lang.text();
    let l = ui.global::<L>();
    l.set_sort(t.sort.into());
    l.set_move_up(t.move_up.into());
    l.set_move_down(t.move_down.into());
    l.set_remove_from_history(t.remove_from_history.into());
    l.set_save_queue(t.save_queue.into());
    l.set_save_queue_title(t.save_queue_title.into());
    l.set_clear_history(t.clear_history.into());
    l.set_clear_history_q(t.clear_history_q.into());
    l.set_clear_history_note(t.clear_history_note.into());
    l.set_home(t.home.into());
    l.set_explore(t.explore.into());
    l.set_library(t.library.into());
    l.set_history(t.history.into());
    l.set_downloads(t.downloads.into());
    l.set_albums(t.albums.into());
    l.set_artists(t.artists.into());
    l.set_playlists(t.playlists.into());
    l.set_songs(t.songs.into());
    l.set_account(t.account.into());
    l.set_sign_in(t.sign_in.into());
    l.set_sign_out(t.sign_out.into());
    l.set_sign_in_google(t.sign_in_google.into());
    l.set_signing_in(t.signing_in.into());
    l.set_syncing(t.syncing.into());
    l.set_searching(t.searching.into());
    l.set_search(t.search.into());
    l.set_search_hint(t.search_hint.into());
    l.set_search_enter(t.search_enter.into());
    l.set_search_prefix(t.search_prefix.into());
    l.set_made_with_slint(t.made_with_slint.into());
    l.set_play(t.play.into());
    l.set_shuffle(t.shuffle.into());
    l.set_radio(t.radio.into());
    l.set_save(t.save.into());
    l.set_saved(t.saved.into());
    l.set_follow(t.follow.into());
    l.set_following(t.following.into());
    l.set_stop(t.stop.into());
    l.set_stop_open(t.stop_open.into());
    l.set_delete_all(t.delete_all.into());
    l.set_show_all(t.show_all.into());
    l.set_songs_suffix(t.songs_suffix.into());
    l.set_nothing_playing(t.nothing_playing.into());
    l.set_playing_from(t.playing_from.into());
    l.set_autoplay(t.autoplay.into());
    l.set_up_next(t.up_next.into());
    l.set_lyrics_tab(t.lyrics_tab.into());
    l.set_related_tab(t.related_tab.into());
    l.set_loading(t.loading.into());
    l.set_loading_video(t.loading_video.into());
    l.set_looking_for_lyrics(t.looking_for_lyrics.into());
    l.set_no_lyrics(t.no_lyrics.into());
    l.set_nothing_found(t.nothing_found.into());
    l.set_empty_playlist(t.empty_playlist.into());
    l.set_empty_history(t.empty_history.into());
    l.set_empty_downloads(t.empty_downloads.into());
    l.set_offline_banner(t.offline_banner.into());
    l.set_offline_empty(t.offline_empty.into());
    l.set_retry(t.retry.into());
    l.set_empty_albums(t.empty_albums.into());
    l.set_empty_artists(t.empty_artists.into());
    l.set_no_match_prefix(t.no_match_prefix.into());
    l.set_no_match_suffix(t.no_match_suffix.into());
    l.set_start_radio(t.start_radio.into());
    l.set_play_next(t.play_next.into());
    l.set_add_to_queue(t.add_to_queue.into());
    l.set_download(t.download.into());
    l.set_remove_download(t.remove_download.into());
    l.set_dislike(t.dislike.into());
    l.set_remove_from_playlist(t.remove_from_playlist.into());
    l.set_save_to_playlist(t.save_to_playlist.into());
    l.set_new_playlist_menu(t.new_playlist_menu.into());
    l.set_new_playlist(t.new_playlist.into());
    l.set_rename_playlist(t.rename_playlist.into());
    l.set_rename(t.rename.into());
    l.set_create(t.create.into());
    l.set_delete(t.delete.into());
    l.set_cancel(t.cancel.into());
    l.set_name(t.name.into());
    l.set_delete_playlist_q_prefix(t.delete_playlist_q_prefix.into());
    l.set_delete_playlist_q_suffix(t.delete_playlist_q_suffix.into());
    l.set_delete_playlist_note(t.delete_playlist_note.into());
    l.set_delete_downloads_q(t.delete_downloads_q.into());
    l.set_delete_downloads_note(t.delete_downloads_note.into());
    l.set_sleep_timer(t.sleep_timer.into());
    l.set_minutes_suffix(t.minutes_suffix.into());
    l.set_end_of_song(t.end_of_song.into());
    l.set_turn_off(t.turn_off.into());
    l.set_play_on(t.play_on.into());
    l.set_this_computer(t.this_computer.into());
    l.set_looking_for_cast(t.looking_for_cast.into());
    l.set_no_cast(t.no_cast.into());
    l.set_settings(t.settings.into());
    l.set_language(t.language.into());
    l.set_version_prefix(t.version_prefix.into());
    l.set_is_available_suffix(t.is_available_suffix.into());
    l.set_up_to_date(t.up_to_date.into());
    l.set_update(t.update.into());
    l.set_updating(t.updating.into());
    l.set_check_now(t.check_now.into());
    l.set_update_to_prefix(t.update_to_prefix.into());
    l.set_autoplay_setting(t.autoplay_setting.into());
    l.set_normalize_setting(t.normalize_setting.into());
    l.set_crossfade_off(t.crossfade_off.into());
    l.set_crossfade_prefix(t.crossfade_prefix.into());
    l.set_seconds_suffix(t.seconds_suffix.into());
    l.set_notifications_setting(t.notifications_setting.into());
    l.set_tray_setting(t.tray_setting.into());
    l.set_youtube_account(t.youtube_account.into());
    l.set_yt_signed_in(t.yt_signed_in.into());
    l.set_yt_sign_in_window(t.yt_sign_in_window.into());
    l.set_yt_sign_in_import(t.yt_sign_in_import.into());
    l.set_import(t.import.into());
    l.set_pick_browser(t.pick_browser.into());
    l.set_browser(t.browser.into());
    l.set_or_browser_sign_in(t.or_browser_sign_in.into());
    l.set_browser_note(t.browser_note.into());
    l.set_off(t.off.into());
    l.set_integrations_note(t.integrations_note.into());
    l.set_google_account(t.google_account.into());
    l.set_yt_signed_in_long(t.yt_signed_in_long.into());
    l.set_yt_sign_in_long_window(t.yt_sign_in_long_window.into());
    l.set_yt_sign_in_long_import(t.yt_sign_in_long_import.into());
    l.set_data_api_title(t.data_api_title.into());
    l.set_data_api_connected(t.data_api_connected.into());
    l.set_data_api_not_needed(t.data_api_not_needed.into());
    l.set_data_api_optional(t.data_api_optional.into());
    l.set_set_up(t.set_up.into());
    l.set_hide(t.hide.into());
    l.set_oauth_client_prefix(t.oauth_client_prefix.into());
    l.set_oauth_step1(t.oauth_step1.into());
    l.set_oauth_step2(t.oauth_step2.into());
    l.set_oauth_how(t.oauth_how.into());
    l.set_open_google_cloud(t.open_google_cloud.into());
    l.set_import_downloaded_json(t.import_downloaded_json.into());
    l.set_or_paste(t.or_paste.into());
    l.set_client_id_hint(t.client_id_hint.into());
    l.set_client_secret_hint(t.client_secret_hint.into());
    l.set_save_client(t.save_client.into());
    l.set_client_set(t.client_set.into());
    l.set_open_browser_again(t.open_browser_again.into());
    l.set_waiting_browser(t.waiting_browser.into());
    l.set_unverified_note(t.unverified_note.into());
    ui.set_lang(lang.code().into());
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

/// The UI-thread view, weakly (for timers it owns).
fn view_weak() -> std::rc::Weak<RefCell<View>> {
    VIEW.with(|v| v.borrow().as_ref().map(Rc::downgrade).unwrap_or_default())
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
    // The size only: a restored position drifted by the title bar's height
    // on every reopen (macOS); the system places the window instead.
    if let Some((size, _)) = view.borrow().geometry {
        ui.window().set_size(size);
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

/// The UI-thread state, before any window.
fn new_view(
    library: Arc<Library>,
    library_view: Rc<RefCell<LibraryView>>,
    prefs: Prefs,
    cmd_tx: UnboundedSender<Cmd>,
) -> Rc<RefCell<View>> {
    let art = Rc::new(RefCell::new(ArtCache {
        thumbs: Images::new(THUMB_CACHE),
        cards: Images::new(CARD_CACHE),
        requested: HashSet::new(),
        large: None,
        tx: cmd_tx.clone(),
    }));
    Rc::new(RefCell::new(View {
        tx: cmd_tx.clone(),
        store: library,
        last: None,
        queue_source: RefCell::default(),
        cast_names: Rc::new(VecModel::default()),
        cast_scanning: false,
        prefs,
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
        lyrics: LyricsView::new(),
        home: None,
        explore: None,
        suggestion_rows: Rc::new(VecModel::default()),
        lang: Lang::default(),
        #[cfg(any(target_os = "macos", windows))]
        video: None,
        #[cfg(any(target_os = "macos", windows))]
        video_rect: (0.0, 0.0, 0.0, 0.0),
        header_art: None,
        pending_add: None,
        art,
        now: None,
    }))
}

/// Runs the GUI until it quits. `rt` runs the core on its own thread.
pub fn run(rt: tokio::runtime::Runtime, deps: Deps, settings: &Settings) -> Result<()> {
    let library = deps.library.clone();
    let lang = Lang::from_code(&settings.language);
    let library_view = Rc::new(RefCell::new(LibraryView::new_in(library.clone(), lang)?));
    let (cmd_tx, cmd_rx) = unbounded_channel();

    let view = new_view(
        library.clone(),
        library_view,
        Prefs {
            cookies: settings.cookies_from_browser.trim().to_lowercase(),
            normalize: settings.normalize_volume,
            notifications: settings.notifications,
            tray: settings.tray,
            crossfade: settings.crossfade,
        },
        cmd_tx.clone(),
    );
    VIEW.with(|cell| *cell.borrow_mut() = Some(view.clone()));
    let library_empty = view.borrow().library.borrow().playlists().is_empty();

    let ui = open_window()?;
    ui.set_autoplay(deps.autoplay);
    // First start: nothing works without an account, so ask right away.
    if !deps.account.signed_in() && !deps.youtube.signed_in() {
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

    // Opening the app again (Finder, Dock) while it runs in the tray.
    #[cfg(target_os = "macos")]
    slint::Timer::single_shot(Duration::ZERO, macos::handle_reopen);

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
        if nav == 7 {
            send(Cmd::LoadExplore, tx);
        }
        navigate(ui, view, &|lib, v| match nav {
            1 => lib.show_home(v.home.as_deref()),
            7 => {
                lib.show_explore(v.explore.as_deref());
                Ok(())
            }
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
    on!(on_lyrics_seek, [tx, view, ui], |ms| {
        if let Ok(ms) = u64::try_from(ms) {
            send(Cmd::SeekTo(Duration::from_millis(ms)), tx);
        }
    });
    on!(on_play_track, [tx, view, ui], |row| {
        // One click plays; the second click of a double-click doesn't
        // start the song over.
        let now = std::time::Instant::now();
        let repeat = LAST_PLAY.replace(Some((row, now))).is_some_and(|(r, at)| {
            r == row && now.duration_since(at) < std::time::Duration::from_millis(500)
        });
        if repeat {
            return;
        }
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
        // One request after the typing stops, not one per keystroke.
        let (query, tx) = (text.to_string(), tx.clone());
        SUGGEST_TIMER.with(|timer| {
            timer.start(slint::TimerMode::SingleShot, SUGGEST_DELAY, move || {
                send(Cmd::Suggest(query.clone()), &tx)
            });
        });
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
                    (Source::History, _) => Cmd::RemoveFromHistory(track.video_id),
                    (_, Some(p)) if row >= 0 => Cmd::RemoveFrom(p.clone(), track),
                    _ => return,
                }
            }
            "up" | "down" => {
                let lib = v.library.borrow();
                let (Some(playlist), Ok(r)) = (lib.shown_playlist(), usize::try_from(row)) else {
                    return;
                };
                let Some((from, _)) = lib.row(r) else { return };
                let to = if action == "up" {
                    from.checked_sub(1)
                } else {
                    Some(from + 1).filter(|&t| t < lib.total())
                };
                let Some(to) = to else { return };
                Cmd::MoveInPlaylist(playlist.clone(), from, to)
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
    // The clip plays in the window on macOS / Windows; elsewhere the
    // browser gets it (`Session::watch_video`).
    on!(on_save_queue_as, [tx, view, ui], |name| {
        send(Cmd::SaveQueue(name.to_string()), tx)
    });
    on!(on_clear_history, [tx, view, ui], || send(
        Cmd::ClearHistory,
        tx
    ));
    on!(on_retry, [tx, view, ui], || send(Cmd::Retry, tx));
    on!(on_cycle_sort, [tx, view, ui], || {
        let mut v = view.borrow_mut();
        {
            let mut lib = v.library.borrow_mut();
            let next = lib.sort().next();
            lib.set_sort(next);
        }
        v.sync_list(ui);
    });
    on!(on_set_language, [tx, view, ui], |code| {
        let lang = Lang::from_code(&code);
        {
            let mut v = view.borrow_mut();
            v.lang = lang;
            v.library.borrow_mut().set_language(lang);
        }
        apply_language(ui, lang);
        // The shown list's own title ("History", "4 songs") is rebuilt the
        // next time it is shown; the sidebar and the rest change now.
        view.borrow_mut().sync_list(ui);
        send(Cmd::SetLanguage(code.into()), tx);
    });
    on!(on_watch_video, [tx, view, ui], || {
        #[cfg(any(target_os = "macos", windows))]
        {
            let mut v = view.borrow_mut();
            if let Some(video) = v.video.take() {
                ui.set_video_on(false);
                send(Cmd::VideoOff(video.position()), tx);
                return;
            }
            if v.last.as_ref().and_then(|s| s.now.as_ref()).is_none() {
                return;
            }
            drop(v);
            // The video needs the big area: open it, and the rectangle's
            // own callback starts the clip once it has a place on screen.
            ui.set_expanded(true);
            ui.set_video_on(true);
            send(Cmd::VideoOn, tx);
        }
        #[cfg(not(any(target_os = "macos", windows)))]
        send(Cmd::WatchVideo, tx);
    });
    #[cfg(any(target_os = "macos", windows))]
    on!(on_video_rect, [tx, view, ui], |x, y, w, h| {
        let mut v = view.borrow_mut();
        v.video_rect = (x, y, w, h);
        if let Some(video) = &v.video {
            return video.set_bounds(v.video_rect);
        }
        if !ui.get_video_on() {
            return;
        }
        let Some((id, at)) = v
            .last
            .as_ref()
            .and_then(|s| s.now.as_ref().map(|t| (t.video_id.clone(), s.position)))
        else {
            return;
        };
        match video::VideoView::open(ui, id, at, v.video_rect) {
            Ok(video) => v.video = Some(video),
            Err(err) => {
                tracing::warn!("opening the video: {err:#}");
                ui.set_video_on(false);
                send(Cmd::VideoOff(None), tx);
            }
        }
    });
    #[cfg(any(target_os = "macos", windows))]
    on!(on_video_off, [tx, view, ui], || {
        let at = view.borrow_mut().video.take().and_then(|v| v.position());
        send(Cmd::VideoOff(at), tx);
    });
    #[cfg(not(any(target_os = "macos", windows)))]
    on!(on_video_rect, [tx, view, ui], |_x, _y, _w, _h| {});
    #[cfg(not(any(target_os = "macos", windows)))]
    on!(on_video_off, [tx, view, ui], || {});
    on!(on_stop_downloads, [tx, view, ui], || send(
        Cmd::StopDownloads,
        tx
    ));
    on!(on_remove_all_downloads, [tx, view, ui], || send(
        Cmd::RemoveAllDownloads,
        tx
    ));
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
    on!(on_yt_sign_in, [tx, view, ui], || send(
        Cmd::YouTubeSignIn(None),
        tx
    ));
    on!(on_yt_sign_out, [tx, view, ui], || send(
        Cmd::YouTubeSignOut,
        tx
    ));
    on!(on_yt_import, [tx, view, ui], || {
        let browser = view.borrow().prefs.cookies.clone();
        if crate::audio::extractor::COOKIE_BROWSERS.contains(&browser.as_str()) {
            send(Cmd::YouTubeSignIn(Some(browser)), tx);
        } else {
            ui.set_status_text("Pick the browser you're signed in to YouTube with first".into());
        }
    });
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
        Cmd::LoadHome => session.load_home(false),
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

/// "23 min" / "end of song" for the sleep timer.
fn sleep_text(sleep: Option<Sleep>, lang: Lang) -> String {
    match sleep {
        None => String::new(),
        Some(Sleep::EndOfTrack) => tr!(lang, "end of song", "кінець пісні").into(),
        Some(Sleep::At(at)) => {
            let left = at.saturating_duration_since(Instant::now()).as_secs();
            trf!(lang, "{} min", "{} хв", left.div_ceil(60))
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

#[cfg(test)]
mod tests;

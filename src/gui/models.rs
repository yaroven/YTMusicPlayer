//! What the window shows, as Slint models and caches on the UI thread:
//! track rows built on demand from the [`LibraryView`] or the queue, rows
//! of cards with their art filled in as it arrives, bounded caches of
//! decoded art, and the karaoke lyrics view.

use super::*;

/// Video id, cover, average colour, blurred backdrop.
pub(super) type LargeArt = (Arc<str>, Image, (u8, u8, u8), Image);

/// Bounded LRU-ish map of decoded images.
pub(super) struct Images {
    pub(super) map: HashMap<Arc<str>, Image>,
    pub(super) order: VecDeque<Arc<str>>,
    pub(super) cap: usize,
}

impl Images {
    pub(super) fn new(cap: usize) -> Self {
        Self {
            map: HashMap::new(),
            order: VecDeque::new(),
            cap,
        }
    }

    pub(super) fn clear(&mut self) {
        self.map = HashMap::new();
        self.order = VecDeque::new();
    }

    pub(super) fn insert(&mut self, key: Arc<str>, image: Image) {
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
pub(super) struct ArtCache {
    /// By video id.
    pub(super) thumbs: Images,
    /// By image URL.
    pub(super) cards: Images,
    /// Requested and not yet answered, or failed: never re-requested.
    pub(super) requested: HashSet<Arc<str>>,
    /// The single large cover (for the playing track).
    pub(super) large: Option<LargeArt>,
    pub(super) tx: UnboundedSender<Cmd>,
}

impl ArtCache {
    /// The image for `key` if decoded, else requests it (once).
    pub(super) fn get(&mut self, key: &Arc<str>, size: ArtSize) -> Option<Image> {
        let images = match size {
            ArtSize::Card | ArtSize::Round => &self.cards,
            _ => &self.thumbs,
        };
        if let Some(image) = images.map.get(key) {
            return Some(image.clone());
        }
        if self.requested.insert(key.clone()) {
            let _ = self
                .tx
                .send(Cmd::FetchArt(key.clone(), size, size.pixels(), None));
        }
        None
    }

    pub(super) fn thumb(&mut self, id: &Arc<str>) -> Option<Image> {
        self.get(id, ArtSize::Thumb)
    }

    pub(super) fn insert(&mut self, key: Arc<str>, size: ArtSize, image: Image) {
        self.requested.remove(&key);
        match size {
            ArtSize::Card | ArtSize::Round => self.cards.insert(key, image),
            _ => self.thumbs.insert(key, image),
        }
    }
}

/// Where a [`TracksModel`] gets its rows.
pub(super) enum Rows {
    /// The library view's (filtered) list.
    Library(Rc<RefCell<LibraryView>>),
    /// The play queue: current track, then what's next.
    Queue(RefCell<Arc<[Track]>>),
}

/// Track rows for a virtualized ListView: built (and their art requested)
/// only when the list asks for them; no track is copied up front.
pub(super) struct TracksModel {
    pub(super) rows: Rows,
    /// Show only the first rows (pages list a few songs above shelves).
    pub(super) limit: std::cell::Cell<Option<usize>>,
    pub(super) playing: RefCell<Option<Arc<str>>>,
    pub(super) art: Rc<RefCell<ArtCache>>,
    pub(super) notify: ModelNotify,
}

impl TracksModel {
    pub(super) fn new(rows: Rows, art: Rc<RefCell<ArtCache>>) -> Self {
        Self {
            rows,
            limit: std::cell::Cell::new(None),
            playing: RefCell::default(),
            art,
            notify: ModelNotify::default(),
        }
    }

    /// All rows changed (new list, new filter).
    pub(super) fn reset(&self) {
        self.notify.reset();
    }

    pub(super) fn set_queue(&self, tracks: Arc<[Track]>) {
        if let Rows::Queue(cell) = &self.rows {
            *cell.borrow_mut() = tracks;
            self.notify.reset();
        }
    }

    pub(super) fn track(&self, row: usize) -> Option<Track> {
        match &self.rows {
            Rows::Library(view) => view.borrow().row(row).map(|(_, t)| t.clone()),
            Rows::Queue(tracks) => tracks.borrow().get(row).cloned(),
        }
    }

    pub(super) fn set_playing(&self, id: Option<Arc<str>>) {
        if *self.playing.borrow() == id {
            return;
        }
        let old = std::mem::replace(&mut *self.playing.borrow_mut(), id.clone());
        for changed in [old, id].into_iter().flatten() {
            self.rows_changed(&changed);
        }
    }

    /// Re-renders rows showing `id` (art arrived, like or playing changed).
    pub(super) fn rows_changed(&self, id: &str) {
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
pub(super) struct Cards {
    pub(super) model: Rc<VecModel<CardRow>>,
    pub(super) keys: Vec<Option<Arc<str>>>,
}

impl Cards {
    pub(super) fn new() -> Self {
        Self {
            model: Rc::new(VecModel::default()),
            keys: Vec::new(),
        }
    }

    pub(super) fn of(items: &[Item], art: &mut ArtCache) -> Self {
        let mut cards = Self::new();
        cards.set(items, art);
        cards
    }

    pub(super) fn set(&mut self, items: &[Item], art: &mut ArtCache) {
        self.keys = items.iter().map(|i| i.thumbnail.clone()).collect();
        self.fill(items, art);
    }

    pub(super) fn fill(&self, items: &[Item], art: &mut ArtCache) {
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

    pub(super) fn art_arrived(&self, key: &Arc<str>, image: &Image) {
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
pub(super) struct Shelves {
    pub(super) model: Rc<VecModel<ShelfRow>>,
    pub(super) rows: Vec<Cards>,
}

impl Shelves {
    pub(super) fn new() -> Self {
        Self {
            model: Rc::new(VecModel::default()),
            rows: Vec::new(),
        }
    }

    pub(super) fn set(&mut self, shelves: &[Shelf], art: &mut ArtCache) {
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

    pub(super) fn art_arrived(&self, key: &Arc<str>, image: &Image) {
        for cards in &self.rows {
            cards.art_arrived(key, image);
        }
    }
}

/// Suggestions the dropdown shows.
pub(super) const SUGGESTIONS_SHOWN: usize = 8;

/// How long typing pauses before suggestions are asked for.
pub(super) const SUGGEST_DELAY: Duration = Duration::from_millis(250);

thread_local! {
    /// Debounces search suggestions (see `on_filter_changed`).
    pub(super) static SUGGEST_TIMER: slint::Timer = slint::Timer::default();
    /// The row last played by a click, and when (see `on_play_track`).
    pub(super) static LAST_PLAY: std::cell::Cell<Option<(i32, std::time::Instant)>> = const { std::cell::Cell::new(None) };
}

/// "chrome", or "cookies.txt" for a file path.
pub(super) fn cookies_label(source: &str) -> &str {
    if source.contains(['/', '\\']) {
        "cookies.txt"
    } else {
        source
    }
}

/// Artists get round pictures.
pub(super) fn card_size(kind: ItemKind) -> ArtSize {
    match kind {
        ItemKind::Artist => ArtSize::Round,
        _ => ArtSize::Card,
    }
}

/// First letter or digit of a title, for the album-art stand-in.
pub(super) fn initial(title: &str) -> SharedString {
    title
        .chars()
        .find(|c| c.is_alphanumeric())
        .map(|c| c.to_uppercase().collect::<String>())
        .unwrap_or_default()
        .into()
}

/// Stable 0..1 hue per artist, so one artist's tiles share a colour.
pub(super) fn hue(artist: &str) -> f32 {
    let h = artist.bytes().fold(0x811c_9dc5_u32, |h, b| {
        (h ^ u32::from(b)).wrapping_mul(0x0100_0193)
    });
    (h % 360) as f32 / 360.0
}

/// Lyrics of the playing track, karaoke style: the sung line lights up
/// word by word (see [`Lyrics::progress`]), instrumental breaks show three
/// dots. A timer redraws only when a word starts, not every frame.
pub(super) struct LyricsView {
    pub(super) lyrics: Option<Arc<Lyrics>>,
    pub(super) model: Rc<VecModel<LyricLine>>,
    /// Model row of lyrics line 0: 1 when an intro row comes first.
    pub(super) offset: usize,
    /// Model row being sung and its words (or dots) lit.
    pub(super) progress: Option<(usize, usize)>,
    /// Asked for and not answered yet.
    pub(super) loading: bool,
    /// Position at the last snapshot, and when it came (`None`: paused).
    pub(super) clock: (Duration, Option<std::time::Instant>),
    pub(super) ticker: slint::Timer,
}

/// How often the timer looks for the next word (no redraw unless one starts).
pub(super) const LYRICS_TICK: Duration = Duration::from_millis(60);

impl LyricsView {
    pub(super) fn new() -> Self {
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

    pub(super) fn set(&mut self, lyrics: Option<Arc<Lyrics>>) {
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
    pub(super) fn follow(&mut self, position: Duration, playing: bool) {
        self.clock = (position, playing.then(std::time::Instant::now));
        self.tick();
    }

    pub(super) fn position(&self) -> Duration {
        let (at, since) = self.clock;
        since.map_or(at, |t| at + t.elapsed())
    }

    /// Lights the words sung by now; true when something changed.
    pub(super) fn tick(&mut self) -> bool {
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
    pub(super) fn schedule(&self, view: std::rc::Weak<RefCell<View>>) {
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

    pub(super) fn render(&self, ui: &MainWindow) {
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
pub(super) fn karaoke_markup(text: &str, lit: usize) -> slint::StyledText {
    pub(super) fn escape(text: &str) -> String {
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

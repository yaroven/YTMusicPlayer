//! Terminal frontend: focus, popups, mouse areas and the main
//! `tokio::select!` loop. What the list shows lives in [`LibraryView`];
//! playback, queue, library changes and browsing in [`Session`]. Redraws
//! only when something changed.

use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::{
    event::{
        DisableMouseCapture, EnableMouseCapture, Event, EventStream, KeyCode, KeyEvent,
        KeyEventKind, MouseButton, MouseEvent, MouseEventKind,
    },
    execute,
};
use futures_util::StreamExt;
use ratatui::{DefaultTerminal, layout::Rect, widgets::ListState};

pub use crate::session::{Deps, Status};
use crate::{
    api::models::{LIKED_PLAYLIST_ID, Playlist, Track},
    catalog::{Item, ItemKind, SearchKind},
    library_view::{LibraryView, SessionData, Source},
    session::{Changes, SEEK_STEP_SECS, Session, Sleep, VOLUME_STEP},
    ui::{self, keymap::Action},
};

const DOUBLE_CLICK: Duration = Duration::from_millis(400);
/// Sleep timer steps for `z` (minutes; `None` = end of track).
const SLEEP_STEPS: [Option<u32>; 4] = [Some(15), Some(30), Some(60), None];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Playlists,
    Tracks,
}

/// What the right-hand pane shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TracksView {
    Playlist,
    Queue,
}

/// What a text prompt is for.
pub enum Prompt {
    NewPlaylist(Option<Track>),
    Rename(Playlist),
}

/// Entries of the playlist menu (`m`) and the cast menu (`C`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuEntry {
    NewPlaylist,
    Rename,
    Delete,
    DownloadAll,
    /// Play on this computer (`None`) or on a found Cast device.
    CastTo(Option<usize>, String),
    /// A line that does nothing ("Looking for devices…").
    Note(&'static str),
}

impl MenuEntry {
    pub fn label(&self) -> &str {
        match self {
            Self::NewPlaylist => "New playlist…",
            Self::Rename => "Rename this playlist…",
            Self::Delete => "Delete this playlist",
            Self::DownloadAll => "Download all for offline",
            Self::CastTo(_, name) => name,
            Self::Note(text) => text,
        }
    }
}

pub enum Mode {
    Normal,
    /// Typing a filter for the list.
    Search,
    /// Typing an online search query.
    Find(String),
    /// Choosing a playlist to add `track` to (last entry: a new one).
    AddTo {
        track: Track,
        state: ListState,
    },
    Prompt {
        purpose: Prompt,
        text: String,
    },
    Menu {
        title: &'static str,
        entries: Vec<MenuEntry>,
        state: ListState,
    },
    Lyrics,
    Help,
}

/// Screen regions from the last draw, for mouse hit-testing.
#[derive(Default, Clone, Copy)]
pub struct Areas {
    pub playlists: Rect,
    pub tracks: Rect,
    pub progress: Rect,
}

pub struct App {
    pub(crate) session: Session,
    pub(crate) library: LibraryView,
    /// Scroll state of the playlist pane (selection mirrors `library`).
    pub(crate) playlist_state: ListState,
    /// First track row on screen.
    pub(crate) track_offset: usize,
    /// Cards (albums, artists) shown instead of the tracks.
    pub(crate) cards: bool,
    pub(crate) card_selected: usize,
    pub(crate) card_offset: usize,
    /// Cursor in the queue view (index among upcoming tracks).
    pub(crate) queue_selected: usize,
    pub(crate) focus: Focus,
    pub(crate) view: TracksView,
    pub(crate) mode: Mode,
    pub(crate) areas: Areas,
    quit: bool,
    last_click: Option<(Instant, usize)>,
}

pub async fn run(deps: Deps) -> Result<()> {
    let library = LibraryView::new(deps.library.clone())?;
    let mut app = App {
        session: Session::new(deps)?,
        library,
        playlist_state: ListState::default(),
        track_offset: 0,
        cards: false,
        card_selected: 0,
        card_offset: 0,
        queue_selected: 0,
        focus: Focus::Playlists,
        view: TracksView::Playlist,
        mode: Mode::Normal,
        areas: Areas::default(),
        quit: false,
        last_click: None,
    };
    app.list_replaced();
    app.session.startup(app.library.playlists().is_empty());

    // Restores the terminal on panic too (installs a panic hook).
    let mut terminal = ratatui::init();
    let _ = execute!(std::io::stdout(), EnableMouseCapture);
    let result = app.event_loop(&mut terminal).await;
    let _ = execute!(std::io::stdout(), DisableMouseCapture);
    ratatui::restore();
    app.library.save();
    app.session.save_state();
    result
}

impl App {
    async fn event_loop(&mut self, terminal: &mut DefaultTerminal) -> Result<()> {
        let mut input = EventStream::new();
        // Progress bar cadence while playing; idle redraws only on input.
        let mut tick = tokio::time::interval(Duration::from_millis(500));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut dirty = true;

        while !self.quit {
            dirty |= self.session.refresh_status();
            if dirty {
                terminal.draw(|frame| ui::draw(frame, self))?;
                dirty = false;
            }

            tokio::select! {
                Some(event) = input.next() => {
                    match event? {
                        // Windows also reports key releases.
                        Event::Key(key) if key.kind != KeyEventKind::Release => self.on_key(key),
                        Event::Mouse(mouse) => self.on_mouse(mouse),
                        _ => {}
                    }
                    dirty = true;
                }
                changes = self.session.next_event() => {
                    self.apply(changes);
                    dirty = true;
                }
                _ = tick.tick() => {}
            }
        }
        self.session.shutdown();
        Ok(())
    }

    /// "This computer", then the Cast devices found.
    fn cast_entries(&self) -> Vec<MenuEntry> {
        let mark = |on: bool, name: &str| format!("{} {name}", if on { "●" } else { " " });
        let casting = self.session.casting.as_deref();
        let mut entries = vec![MenuEntry::CastTo(
            None,
            mark(casting.is_none(), "This computer"),
        )];
        for (i, d) in self.session.cast_devices.iter().enumerate() {
            entries.push(MenuEntry::CastTo(
                Some(i),
                mark(casting == Some(d.name.as_str()), &d.name),
            ));
        }
        if self.session.cast_devices.is_empty() {
            entries.push(MenuEntry::Note(if self.session.cast_scanning {
                "  Looking for Chromecasts…"
            } else {
                "  No Chromecasts found"
            }));
        }
        entries
    }

    /// Reloads whatever the session changed.
    fn apply(&mut self, changes: Changes) {
        if changes.cast
            && let Mode::Menu {
                title: " Play on ", ..
            } = self.mode
        {
            let entries = self.cast_entries();
            if let Mode::Menu { entries: e, .. } = &mut self.mode {
                *e = entries;
            }
        }
        let data = SessionData {
            search: self.session.search.as_ref(),
            page: self.session.page.as_deref(),
            home: self.session.home.as_deref(),
        };
        match self.library.apply(&changes, data) {
            Ok(true) => {
                if changes.search || changes.page {
                    self.view = TracksView::Playlist;
                    self.focus = Focus::Tracks;
                }
                self.list_replaced();
            }
            Ok(false) => {}
            Err(err) => self.session.set_error(format!("library: {err:#}")),
        }
    }

    /// The list got new rows: a cursor at the top, scrolled up, cards off,
    /// and the playlist pane pointing at the shown playlist.
    fn list_replaced(&mut self) {
        self.library.ensure_selection();
        self.track_offset = 0;
        self.cards = false;
        self.card_selected = 0;
        self.card_offset = 0;
        if let Some(i) = self.library.selected_playlist() {
            self.playlist_state.select(Some(i));
        }
    }

    // --- cards (albums, artists, playlists) ---------------------------------------

    /// Number of cards: the list's own items, else the page's shelves.
    pub(crate) fn card_count(&self) -> usize {
        match self.library.items().len() {
            0 => self.library.shelves().iter().map(|s| s.items.len()).sum(),
            n => n,
        }
    }

    /// Card `i` with the title of its shelf ("" for plain lists).
    pub(crate) fn card(&self, i: usize) -> Option<(&str, &Item)> {
        if !self.library.items().is_empty() {
            return self.library.items().get(i).map(|item| ("", item));
        }
        let mut i = i;
        for shelf in self.library.shelves().iter() {
            if i < shelf.items.len() {
                return Some((shelf.title.as_str(), &shelf.items[i]));
            }
            i -= shelf.items.len();
        }
        None
    }

    /// The pane shows cards: there are no tracks, or `i` switched to them.
    pub(crate) fn showing_cards(&self) -> bool {
        self.view == TracksView::Playlist
            && self.card_count() > 0
            && (self.cards || self.library.total() == 0)
    }

    fn open_card(&mut self, i: usize) {
        let Some((_, item)) = self.card(i) else {
            return;
        };
        match &item.track {
            Some(track) => {
                let track = track.clone();
                self.session.start_radio(track);
            }
            None => {
                let id = item.id.clone();
                self.session.open_page(&id);
            }
        }
    }

    // --- input -----------------------------------------------------------------

    fn on_key(&mut self, key: KeyEvent) {
        match &mut self.mode {
            Mode::Help => {
                self.mode = Mode::Normal;
                return;
            }
            Mode::Lyrics => {
                if matches!(key.code, KeyCode::Esc | KeyCode::Char('t' | 'q')) {
                    self.mode = Mode::Normal;
                    self.session.want_lyrics(false);
                }
                return;
            }
            Mode::Search => {
                let mut filter = self.library.filter().to_owned();
                match key.code {
                    KeyCode::Esc => {
                        filter.clear();
                        self.mode = Mode::Normal;
                    }
                    KeyCode::Enter => {
                        self.mode = Mode::Normal;
                        self.focus = Focus::Tracks;
                        return;
                    }
                    KeyCode::Backspace => {
                        filter.pop();
                    }
                    KeyCode::Char(c) => filter.push(c),
                    _ => return,
                }
                self.library.set_filter(&filter);
                self.library.ensure_selection();
                self.track_offset = 0;
                return;
            }
            Mode::Find(query) => {
                match key.code {
                    KeyCode::Esc => self.mode = Mode::Normal,
                    KeyCode::Enter => {
                        let query = std::mem::take(query);
                        self.mode = Mode::Normal;
                        self.session.search(&query, SearchKind::Songs);
                    }
                    KeyCode::Backspace => {
                        query.pop();
                    }
                    KeyCode::Char(c) => query.push(c),
                    _ => {}
                }
                return;
            }
            Mode::Prompt { text, .. } => {
                match key.code {
                    KeyCode::Esc => self.mode = Mode::Normal,
                    KeyCode::Enter => self.confirm_prompt(),
                    KeyCode::Backspace => {
                        text.pop();
                    }
                    KeyCode::Char(c) => text.push(c),
                    _ => {}
                }
                return;
            }
            Mode::Menu { entries, state, .. } => {
                let len = entries.len();
                match key.code {
                    KeyCode::Esc | KeyCode::Char('q') => self.mode = Mode::Normal,
                    KeyCode::Up | KeyCode::Char('k') => state.select_previous(),
                    KeyCode::Down | KeyCode::Char('j') => {
                        let next = state.selected().map_or(0, |i| (i + 1).min(len - 1));
                        state.select(Some(next));
                    }
                    KeyCode::Enter => self.confirm_menu(),
                    _ => {}
                }
                return;
            }
            Mode::AddTo { state, .. } => {
                // The choices, then "New playlist…".
                let len = self.library.add_choices().count() + 1;
                match key.code {
                    KeyCode::Esc | KeyCode::Char('q') => self.mode = Mode::Normal,
                    KeyCode::Up | KeyCode::Char('k') => state.select_previous(),
                    KeyCode::Down | KeyCode::Char('j') => {
                        let next = state.selected().map_or(0, |i| (i + 1).min(len - 1));
                        state.select(Some(next));
                    }
                    KeyCode::Enter => self.confirm_add_to(),
                    _ => {}
                }
                return;
            }
            Mode::Normal => {}
        }
        if let Some(action) = ui::keymap::action_for(key) {
            self.on_action(action);
        }
    }

    fn on_action(&mut self, action: Action) {
        let s = &mut self.session;
        match action {
            Action::Quit => self.quit = true,
            Action::Help => self.mode = Mode::Help,
            Action::Up => self.move_selection(-1),
            Action::Down => self.move_selection(1),
            Action::PageUp => self.move_selection(-10),
            Action::PageDown => self.move_selection(10),
            Action::Top => self.move_selection(i64::MIN / 2),
            Action::Bottom => self.move_selection(i64::MAX / 2),
            Action::FocusNext => {
                self.focus = match self.focus {
                    Focus::Playlists => Focus::Tracks,
                    Focus::Tracks => Focus::Playlists,
                }
            }
            Action::Select => self.select(),
            Action::Back => {
                if self.view == TracksView::Queue {
                    self.view = TracksView::Playlist;
                } else if self.library.back() {
                    self.list_replaced();
                }
            }
            Action::TogglePause => s.toggle_pause(),
            Action::Next => s.skip(1),
            Action::Prev => s.skip(-1),
            Action::SeekBack => s.seek_by(-SEEK_STEP_SECS),
            Action::SeekForward => s.seek_by(SEEK_STEP_SECS),
            Action::VolumeUp => s.change_volume(VOLUME_STEP),
            Action::VolumeDown => s.change_volume(-VOLUME_STEP),
            Action::Sync => s.start_sync(),
            Action::Search => {
                self.view = TracksView::Playlist;
                self.mode = Mode::Search;
            }
            Action::FindOnline => self.mode = Mode::Find(String::new()),
            Action::CategoryPrev | Action::CategoryNext => {
                let (Some(query), Some(kind)) = (
                    self.library.search_query().map(str::to_owned),
                    self.library.search_kind(),
                ) else {
                    return;
                };
                let all = SearchKind::ALL;
                let i = all.iter().position(|k| *k == kind).unwrap_or(0);
                let next = if action == Action::CategoryNext {
                    all[(i + 1) % all.len()]
                } else {
                    all[(i + all.len() - 1) % all.len()]
                };
                self.session.search(&query, next);
            }
            Action::ToggleCards => {
                if self.card_count() > 0 && self.library.total() > 0 {
                    self.cards = !self.cards;
                }
            }
            Action::Shuffle => s.toggle_shuffle(),
            Action::Repeat => s.cycle_repeat(),
            Action::Autoplay => s.toggle_autoplay(),
            Action::ToggleQueue => {
                self.view = match self.view {
                    TracksView::Playlist => TracksView::Queue,
                    TracksView::Queue => TracksView::Playlist,
                };
                self.queue_selected = 0;
                self.focus = Focus::Tracks;
            }
            Action::PlayNext => {
                if let Some(track) = self.action_target() {
                    self.session.play_next(track);
                }
            }
            Action::Radio => {
                if let Some(track) = self.action_target() {
                    self.session.start_radio(track);
                }
            }
            Action::Artist => {
                if let Some(track) = self.action_target() {
                    self.session.open_artist(&track);
                }
            }
            Action::Like => {
                if let Some(track) = self.action_target() {
                    self.session.toggle_like(track);
                }
            }
            Action::Dislike => {
                if let Some(track) = self.action_target() {
                    self.session.dislike(track);
                }
            }
            Action::AddToPlaylist => self.open_add_to(),
            Action::Remove => self.remove(),
            Action::MoveUp | Action::MoveDown if self.view == TracksView::Queue => {
                let from = self.queue_selected;
                let to = if action == Action::MoveUp {
                    from.saturating_sub(1)
                } else {
                    from + 1
                };
                if to != from && to < self.upcoming_len() {
                    self.session.move_in_queue(from, to);
                    self.queue_selected = to;
                }
            }
            Action::MoveUp | Action::MoveDown => {}
            Action::ClearQueue => {
                self.session.clear_queue();
                self.queue_selected = 0;
            }
            Action::Download => {
                if let Some(track) = self.action_target() {
                    self.session.download(vec![track]);
                }
            }
            Action::SaveOrFollow => {
                let item = if self.showing_cards() {
                    self.card(self.card_selected)
                        .map(|(_, i)| (i.clone(), None))
                } else {
                    self.library.page_item()
                };
                match item {
                    Some((item, channel)) if item.kind != ItemKind::Song => {
                        self.session.toggle_saved(item, channel)
                    }
                    _ => self.session.set_error("Open an album or artist to save it"),
                }
            }
            Action::PlaylistMenu => {
                let mut entries = vec![MenuEntry::NewPlaylist];
                if self
                    .library
                    .shown_playlist()
                    .is_some_and(|p| p.id != LIKED_PLAYLIST_ID)
                {
                    entries.extend([MenuEntry::Rename, MenuEntry::Delete]);
                }
                if self.library.total() > 0 {
                    entries.push(MenuEntry::DownloadAll);
                }
                let mut state = ListState::default();
                state.select(Some(0));
                self.mode = Mode::Menu {
                    title: " Playlist ",
                    entries,
                    state,
                };
            }
            Action::Lyrics => {
                self.session.want_lyrics(true);
                self.mode = Mode::Lyrics;
            }
            Action::Sleep => self.cycle_sleep(),
            Action::Cast => {
                self.session.find_cast_devices();
                self.mode = Mode::Menu {
                    title: " Play on ",
                    entries: self.cast_entries(),
                    state: ListState::default().with_selected(Some(0)),
                };
            }
            Action::Home => {
                self.session.load_home(false);
                let home = self.session.home.clone();
                self.navigate(|lib| lib.show_home(home.as_deref()));
            }
            Action::History => self.navigate(LibraryView::show_history),
            Action::Downloads => self.navigate(LibraryView::show_downloads),
            Action::Albums => self.navigate(|lib| lib.show_saved(ItemKind::Album)),
            Action::Artists => self.navigate(|lib| lib.show_saved(ItemKind::Artist)),
            Action::SignIn => self.session.sign_in(),
        }
    }

    /// Off → 15 → 30 → 60 min → end of track → off.
    fn cycle_sleep(&mut self) {
        let step = match self.session.sleep {
            None => Some(SLEEP_STEPS[0]),
            Some(Sleep::EndOfTrack) => None,
            Some(Sleep::At(at)) => {
                let left = at.saturating_duration_since(Instant::now()).as_secs() / 60;
                // The first step longer than what's left (else end of track).
                Some(
                    SLEEP_STEPS
                        .iter()
                        .copied()
                        .find(|s| s.is_some_and(|m| u64::from(m) > left + 1))
                        .unwrap_or(None),
                )
            }
        };
        match step {
            Some(step) => self.session.set_sleep(step),
            None => self.session.cancel_sleep(),
        }
    }

    fn navigate(&mut self, show: impl FnOnce(&mut LibraryView) -> Result<()>) {
        if let Err(err) = show(&mut self.library) {
            self.session.set_error(format!("library: {err:#}"));
        }
        self.view = TracksView::Playlist;
        self.focus = Focus::Tracks;
        self.list_replaced();
    }

    fn upcoming_len(&self) -> usize {
        self.session.queue.upcoming().count()
    }

    /// `x`: out of the queue, the shown playlist, or the downloads.
    fn remove(&mut self) {
        if self.view == TracksView::Queue {
            self.session.remove_from_queue(self.queue_selected);
            self.queue_selected = self
                .queue_selected
                .min(self.upcoming_len().saturating_sub(1));
            return;
        }
        let Some(track) = self.library.selected_track().cloned() else {
            return;
        };
        if let Some(playlist) = self.library.shown_playlist().cloned() {
            self.session.remove_from_playlist(playlist, track);
        } else if *self.library.source() == Source::Downloads {
            self.session.remove_download(track.video_id);
        }
    }

    fn on_mouse(&mut self, mouse: MouseEvent) {
        tracing::trace!(?mouse, progress = ?self.areas.progress, "mouse");
        if !matches!(self.mode, Mode::Normal) {
            return;
        }
        let (x, y) = (mouse.column, mouse.row);
        let hit = |r: Rect| x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height;
        let Areas {
            playlists,
            tracks,
            progress,
        } = self.areas;
        match mouse.kind {
            MouseEventKind::ScrollDown | MouseEventKind::ScrollUp => {
                let delta = if mouse.kind == MouseEventKind::ScrollDown {
                    3
                } else {
                    -3
                };
                if hit(playlists) {
                    self.focus = Focus::Playlists;
                } else if hit(tracks) {
                    self.focus = Focus::Tracks;
                } else {
                    return;
                }
                self.move_selection(delta);
            }
            MouseEventKind::Down(MouseButton::Left) if hit(progress) => {
                let ratio = f64::from(x - progress.x) / f64::from(progress.width.max(1));
                self.session.seek_ratio(ratio);
            }
            MouseEventKind::Down(MouseButton::Left) if hit(playlists) => {
                self.focus = Focus::Playlists;
                // Row 0 is the border.
                let row = (y - playlists.y) as usize;
                if row >= 1 {
                    let index = self.playlist_state.offset() + row - 1;
                    if index < self.library.playlists().len()
                        && Some(index) != self.library.selected_playlist()
                    {
                        self.show_playlist(index);
                    }
                }
            }
            MouseEventKind::Down(MouseButton::Left) if hit(tracks) => {
                self.focus = Focus::Tracks;
                // Border + header row, then rows.
                let row = (y - tracks.y) as usize;
                if row < 2 || self.view != TracksView::Playlist {
                    return;
                }
                let cards = self.showing_cards();
                let offset = if cards {
                    self.card_offset
                } else {
                    self.track_offset
                };
                let index = row - 2 + offset;
                let len = if cards {
                    self.card_count()
                } else {
                    self.library.len()
                };
                if index >= len {
                    return;
                }
                let double = self
                    .last_click
                    .is_some_and(|(t, i)| i == index && t.elapsed() < DOUBLE_CLICK);
                self.last_click = Some((Instant::now(), index));
                if cards {
                    self.card_selected = index;
                    if double {
                        self.open_card(index);
                    }
                } else {
                    self.library.select_row(Some(index));
                    if double {
                        self.play_selected();
                    }
                }
            }
            _ => {}
        }
    }

    // --- navigation ------------------------------------------------------------

    fn show_playlist(&mut self, index: usize) {
        if let Err(err) = self.library.select_playlist(Some(index)) {
            self.session.set_error(format!("library: {err:#}"));
        }
        self.view = TracksView::Playlist;
        self.list_replaced();
    }

    fn move_selection(&mut self, delta: i64) {
        let clamp = |from: usize, len: usize| {
            (from as i64)
                .saturating_add(delta)
                .clamp(0, len.saturating_sub(1) as i64) as usize
        };
        match self.focus {
            Focus::Playlists if !self.library.playlists().is_empty() => {
                let from = self
                    .library
                    .selected_playlist()
                    .or(self.playlist_state.selected())
                    .unwrap_or(0);
                let target = clamp(from, self.library.playlists().len());
                if Some(target) != self.library.selected_playlist() {
                    self.show_playlist(target);
                }
            }
            Focus::Tracks if self.view == TracksView::Queue => {
                self.queue_selected = clamp(self.queue_selected, self.upcoming_len());
            }
            Focus::Tracks if self.showing_cards() => {
                self.card_selected = clamp(self.card_selected, self.card_count());
            }
            Focus::Tracks => self.library.move_selection(delta),
            _ => {}
        }
    }

    fn select(&mut self) {
        match self.focus {
            Focus::Playlists => {
                if self.library.selected_playlist().is_none()
                    && let Some(i) = self.playlist_state.selected()
                {
                    self.show_playlist(i);
                }
                self.view = TracksView::Playlist;
                self.focus = Focus::Tracks;
            }
            Focus::Tracks if self.view == TracksView::Queue => {
                self.session.skip_ahead(self.queue_selected);
                self.queue_selected = 0;
            }
            Focus::Tracks if self.showing_cards() => self.open_card(self.card_selected),
            Focus::Tracks => self.play_selected(),
        }
    }

    fn play_selected(&mut self) {
        if let Some((tracks, index)) = self.library.play_from(self.library.selected_row()) {
            self.session.play(tracks, index);
        }
    }

    /// What like / play next / add-to act on: in the track list, the
    /// library view's target; in the queue, the selected upcoming track;
    /// elsewhere the playing track.
    fn action_target(&self) -> Option<Track> {
        let playing = self.session.queue.current();
        match (self.focus, self.view) {
            (Focus::Tracks, TracksView::Queue) => self
                .session
                .queue
                .upcoming()
                .nth(self.queue_selected)
                .cloned(),
            (Focus::Tracks, TracksView::Playlist) if self.showing_cards() => self
                .card(self.card_selected)
                .and_then(|(_, i)| i.track.clone())
                .or_else(|| playing.cloned()),
            (Focus::Tracks, TracksView::Playlist) => self.library.target(playing),
            _ => playing.cloned(),
        }
    }

    fn open_add_to(&mut self) {
        if !self.session.can_edit() {
            return;
        }
        let Some(track) = self.action_target() else {
            return;
        };
        let mut state = ListState::default();
        state.select(Some(0));
        self.mode = Mode::AddTo { track, state };
    }

    fn confirm_add_to(&mut self) {
        let Mode::AddTo { track, state } = std::mem::replace(&mut self.mode, Mode::Normal) else {
            return;
        };
        let i = state.selected().unwrap_or(0);
        match self.library.add_choice(i) {
            Some(playlist) => self.session.add_to_playlist(playlist.clone(), track),
            None => {
                self.mode = Mode::Prompt {
                    purpose: Prompt::NewPlaylist(Some(track)),
                    text: String::new(),
                }
            }
        }
    }

    fn confirm_menu(&mut self) {
        let Mode::Menu { entries, state, .. } = std::mem::replace(&mut self.mode, Mode::Normal)
        else {
            return;
        };
        let Some(entry) = state.selected().and_then(|i| entries.get(i)).cloned() else {
            return;
        };
        let shown = self.library.shown_playlist().cloned();
        match (entry, shown) {
            (MenuEntry::NewPlaylist, _) => {
                self.mode = Mode::Prompt {
                    purpose: Prompt::NewPlaylist(None),
                    text: String::new(),
                }
            }
            (MenuEntry::Rename, Some(playlist)) => {
                self.mode = Mode::Prompt {
                    text: playlist.title.clone(),
                    purpose: Prompt::Rename(playlist),
                }
            }
            (MenuEntry::Delete, Some(playlist)) => self.session.delete_playlist(playlist),
            (MenuEntry::DownloadAll, _) => {
                if let Some(tracks) = self.library.all_tracks() {
                    self.session.download(tracks.to_vec());
                }
            }
            (MenuEntry::CastTo(index, _), _) => {
                let device = index.and_then(|i| self.session.cast_devices.get(i).cloned());
                self.session.cast_to(device);
            }
            _ => {}
        }
    }

    fn confirm_prompt(&mut self) {
        let Mode::Prompt { purpose, text } = std::mem::replace(&mut self.mode, Mode::Normal) else {
            return;
        };
        match purpose {
            Prompt::NewPlaylist(track) => self.session.create_playlist(&text, track),
            Prompt::Rename(playlist) => self.session.rename_playlist(playlist, &text),
        }
    }
}

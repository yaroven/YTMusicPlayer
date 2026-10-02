//! Terminal frontend: focus, popups, mouse areas and the main
//! `tokio::select!` loop. What the track list shows lives in
//! [`LibraryView`]; playback, queue and library changes in [`Session`].
//! Redraws only when something changed.

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
    api::models::Track,
    library_view::LibraryView,
    session::{Changes, SEEK_STEP_SECS, Session, VOLUME_STEP},
    ui::{self, keymap::Action},
};

const DOUBLE_CLICK: Duration = Duration::from_millis(400);

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

pub enum Mode {
    Normal,
    /// Typing a filter for the track list.
    Search,
    /// Typing an online search query.
    Find(String),
    /// Choosing a playlist to add `track` to.
    AddTo {
        track: Track,
        state: ListState,
    },
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

    /// Reloads whatever the session changed.
    fn apply(&mut self, changes: Changes) {
        match self.library.apply(&changes, self.session.search.as_ref()) {
            Ok(true) => {
                if changes.search {
                    self.view = TracksView::Playlist;
                    self.focus = Focus::Tracks;
                }
                self.list_replaced();
            }
            Ok(false) => {}
            Err(err) => self.session.set_error(format!("library: {err:#}")),
        }
    }

    /// The track list got new rows: a cursor at the top, scrolled up, and
    /// the playlist pane pointing at the shown playlist.
    fn list_replaced(&mut self) {
        self.library.ensure_selection();
        self.track_offset = 0;
        self.playlist_state.select(self.library.selected_playlist());
    }

    // --- input -----------------------------------------------------------------

    fn on_key(&mut self, key: KeyEvent) {
        match &mut self.mode {
            Mode::Help => {
                self.mode = Mode::Normal;
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
                        self.session.search(&query);
                    }
                    KeyCode::Backspace => {
                        query.pop();
                    }
                    KeyCode::Char(c) => query.push(c),
                    _ => {}
                }
                return;
            }
            Mode::AddTo { state, .. } => {
                let len = self.library.add_choices().count();
                match key.code {
                    KeyCode::Esc | KeyCode::Char('q') => self.mode = Mode::Normal,
                    KeyCode::Up | KeyCode::Char('k') => state.select_previous(),
                    KeyCode::Down | KeyCode::Char('j') => {
                        let next = state
                            .selected()
                            .map_or(0, |i| (i + 1).min(len.saturating_sub(1)));
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
            Action::Shuffle => s.toggle_shuffle(),
            Action::Repeat => s.cycle_repeat(),
            Action::ToggleQueue => {
                self.view = match self.view {
                    TracksView::Playlist => TracksView::Queue,
                    TracksView::Queue => TracksView::Playlist,
                };
                self.focus = Focus::Tracks;
            }
            Action::PlayNext => {
                if let Some(track) = self.action_target() {
                    self.session.play_next(track);
                }
            }
            Action::Like => {
                if let Some(track) = self.action_target() {
                    self.session.toggle_like(track);
                }
            }
            Action::AddToPlaylist => self.open_add_to(),
            Action::SignIn => self.session.sign_in(),
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
                // Border + header row, then tracks.
                let row = (y - tracks.y) as usize;
                if row >= 2 && self.view == TracksView::Playlist {
                    let index = self.track_offset + row - 2;
                    if index < self.library.len() {
                        self.library.select_row(Some(index));
                        let double = self
                            .last_click
                            .is_some_and(|(t, i)| i == index && t.elapsed() < DOUBLE_CLICK);
                        self.last_click = Some((Instant::now(), index));
                        if double {
                            self.play_selected();
                        }
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
        self.list_replaced();
    }

    fn move_selection(&mut self, delta: i64) {
        match self.focus {
            Focus::Playlists if !self.library.playlists().is_empty() => {
                let last = self.library.playlists().len() as i64 - 1;
                let from = self.library.selected_playlist().map_or(0, |i| i as i64);
                let target = from.saturating_add(delta).clamp(0, last) as usize;
                if Some(target) != self.library.selected_playlist() {
                    self.show_playlist(target);
                }
            }
            Focus::Tracks if self.view == TracksView::Playlist => {
                self.library.move_selection(delta);
            }
            _ => {}
        }
    }

    fn select(&mut self) {
        match self.focus {
            Focus::Playlists => {
                self.view = TracksView::Playlist;
                if !self.library.is_empty() {
                    self.focus = Focus::Tracks;
                }
            }
            Focus::Tracks if self.view == TracksView::Playlist => self.play_selected(),
            Focus::Tracks => {}
        }
    }

    fn play_selected(&mut self) {
        if let Some((tracks, index)) = self.library.play_from(self.library.selected_row()) {
            self.session.play(tracks, index);
        }
    }

    /// What like / play next / add-to act on: in the track list, the
    /// library view's target; elsewhere the playing track.
    fn action_target(&self) -> Option<Track> {
        let playing = self.session.queue.current();
        match (self.focus, self.view) {
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
        if self.library.add_choices().next().is_none() {
            self.session.set_error("You have no playlists to add to");
            return;
        }
        let mut state = ListState::default();
        state.select(Some(0));
        self.mode = Mode::AddTo { track, state };
    }

    fn confirm_add_to(&mut self) {
        let Mode::AddTo { track, state } = std::mem::replace(&mut self.mode, Mode::Normal) else {
            return;
        };
        if let Some(playlist) = state.selected().and_then(|i| self.library.add_choice(i)) {
            self.session.add_to_playlist(playlist.clone(), track);
        }
    }
}

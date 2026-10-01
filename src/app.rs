//! Terminal frontend: TUI state (selection, filter, popups, mouse areas) and
//! the main `tokio::select!` loop. Playback, queue and library changes live
//! in [`Session`]. Redraws only when something changed.

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

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
    pub(crate) playlists: Vec<Playlist>,
    pub(crate) playlist_state: ListState,
    /// Tracks of the selected playlist, shared with the queue.
    pub(crate) tracks: Arc<[Track]>,
    /// Indices into `tracks` matching the filter (all when no filter).
    pub(crate) visible: Vec<u32>,
    pub(crate) filter: String,
    /// Selected row in `visible`, and the first row on screen.
    pub(crate) track_selected: Option<usize>,
    pub(crate) track_offset: usize,
    pub(crate) focus: Focus,
    pub(crate) view: TracksView,
    pub(crate) mode: Mode,
    pub(crate) areas: Areas,
    quit: bool,
    last_click: Option<(Instant, usize)>,
}

pub async fn run(deps: Deps) -> Result<()> {
    let mut app = App {
        session: Session::new(deps)?,
        playlists: Vec::new(),
        playlist_state: ListState::default(),
        tracks: Arc::from([]),
        visible: Vec::new(),
        filter: String::new(),
        track_selected: None,
        track_offset: 0,
        focus: Focus::Playlists,
        view: TracksView::Playlist,
        mode: Mode::Normal,
        areas: Areas::default(),
        quit: false,
        last_click: None,
    };
    let last = app.session.last_playlist();
    app.reload_playlists(last.as_deref());
    app.session.startup(app.playlists.is_empty());

    // Restores the terminal on panic too (installs a panic hook).
    let mut terminal = ratatui::init();
    let _ = execute!(std::io::stdout(), EnableMouseCapture);
    let result = app.event_loop(&mut terminal).await;
    let _ = execute!(std::io::stdout(), DisableMouseCapture);
    ratatui::restore();
    let selected = app.selected_playlist().map(|p| p.id.clone());
    app.session.save_state(selected.as_deref());
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
        if changes.library {
            self.reload_playlists(None);
        }
        if let Some(id) = changes.playlist {
            self.refresh_after_edit(&id);
        }
    }

    // --- input -----------------------------------------------------------------

    fn on_key(&mut self, key: KeyEvent) {
        match &mut self.mode {
            Mode::Help => {
                self.mode = Mode::Normal;
                return;
            }
            Mode::Search => {
                match key.code {
                    KeyCode::Esc => {
                        self.filter.clear();
                        self.mode = Mode::Normal;
                    }
                    KeyCode::Enter => {
                        self.mode = Mode::Normal;
                        self.focus = Focus::Tracks;
                        return;
                    }
                    KeyCode::Backspace => {
                        self.filter.pop();
                    }
                    KeyCode::Char(c) => self.filter.push(c),
                    _ => return,
                }
                self.apply_filter();
                return;
            }
            Mode::AddTo { state, .. } => {
                let len = self
                    .playlists
                    .iter()
                    .filter(|p| p.id != LIKED_PLAYLIST_ID)
                    .count();
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
                if let Some(track) = self.selected_track().cloned() {
                    self.session.play_next(track);
                }
            }
            Action::Like => {
                if let Some(track) = self.action_target() {
                    self.session.toggle_like(track);
                }
            }
            Action::AddToPlaylist => self.open_add_to(),
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
                    if index < self.playlists.len() && Some(index) != self.playlist_state.selected()
                    {
                        self.playlist_state.select(Some(index));
                        self.load_tracks();
                    }
                }
            }
            MouseEventKind::Down(MouseButton::Left) if hit(tracks) => {
                self.focus = Focus::Tracks;
                // Border + header row, then tracks.
                let row = (y - tracks.y) as usize;
                if row >= 2 && self.view == TracksView::Playlist {
                    let index = self.track_offset + row - 2;
                    if index < self.visible.len() {
                        self.track_selected = Some(index);
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

    fn selected_playlist(&self) -> Option<&Playlist> {
        self.playlist_state
            .selected()
            .and_then(|i| self.playlists.get(i))
    }

    fn selected_track(&self) -> Option<&Track> {
        let row = self.track_selected?;
        self.tracks.get(*self.visible.get(row)? as usize)
    }

    fn move_selection(&mut self, delta: i64) {
        let clamp = |current: Option<usize>, len: usize| {
            (current.unwrap_or(0) as i64)
                .saturating_add(delta)
                .clamp(0, len as i64 - 1) as usize
        };
        match self.focus {
            Focus::Playlists if !self.playlists.is_empty() => {
                let target = clamp(self.playlist_state.selected(), self.playlists.len());
                if Some(target) != self.playlist_state.selected() {
                    self.playlist_state.select(Some(target));
                    self.load_tracks();
                }
            }
            Focus::Tracks if self.view == TracksView::Playlist && !self.visible.is_empty() => {
                self.track_selected = Some(clamp(self.track_selected, self.visible.len()));
            }
            _ => {}
        }
    }

    fn select(&mut self) {
        match self.focus {
            Focus::Playlists => {
                self.view = TracksView::Playlist;
                if !self.visible.is_empty() {
                    self.focus = Focus::Tracks;
                }
            }
            Focus::Tracks if self.view == TracksView::Playlist => self.play_selected(),
            Focus::Tracks => {}
        }
    }

    fn play_selected(&mut self) {
        let Some(&index) = self.track_selected.and_then(|row| self.visible.get(row)) else {
            return;
        };
        self.session.play(self.tracks.clone(), index as usize);
    }

    fn reload_playlists(&mut self, prefer_id: Option<&str>) {
        let keep = prefer_id
            .map(str::to_owned)
            .or_else(|| self.selected_playlist().map(|p| p.id.clone()));
        match self.session.library().playlists() {
            Ok(playlists) => self.playlists = playlists,
            Err(err) => self.session.set_error(format!("library: {err:#}")),
        }
        let index = keep
            .and_then(|id| self.playlists.iter().position(|p| p.id == id))
            .or((!self.playlists.is_empty()).then_some(0));
        self.playlist_state.select(index);
        self.load_tracks();
    }

    fn load_tracks(&mut self) {
        let Some(id) = self.selected_playlist().map(|p| p.id.clone()) else {
            self.tracks = Arc::from([]);
            self.apply_filter();
            return;
        };
        match self.session.library().tracks(&id) {
            Ok(tracks) => self.tracks = tracks,
            Err(err) => self.session.set_error(format!("library: {err:#}")),
        }
        self.filter.clear();
        self.apply_filter();
    }

    /// Rebuilds `visible` from `filter` (case-insensitive title/artist match).
    fn apply_filter(&mut self) {
        filter_indices(&self.tracks, &self.filter, &mut self.visible);
        self.track_selected = (!self.visible.is_empty()).then_some(0);
        self.track_offset = 0;
    }

    /// Target of like / add-to-playlist: the selected row in the track list,
    /// else the playing track.
    fn action_target(&self) -> Option<Track> {
        match (self.focus, self.view) {
            (Focus::Tracks, TracksView::Playlist) => self.selected_track().cloned(),
            _ => self.session.queue.current().cloned(),
        }
    }

    fn open_add_to(&mut self) {
        if !self.session.can_edit() {
            return;
        }
        let Some(track) = self.action_target() else {
            return;
        };
        if !self.playlists.iter().any(|p| p.id != LIKED_PLAYLIST_ID) {
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
        let playlist = state.selected().and_then(|i| {
            self.playlists
                .iter()
                .filter(|p| p.id != LIKED_PLAYLIST_ID)
                .nth(i)
                .cloned()
        });
        if let Some(playlist) = playlist {
            self.session.add_to_playlist(playlist, track);
        }
    }

    /// Reloads counts, and the track list if it shows the edited playlist.
    fn refresh_after_edit(&mut self, playlist_id: &str) {
        let viewing = self
            .selected_playlist()
            .is_some_and(|p| p.id == playlist_id);
        if let Ok(playlists) = self.session.library().playlists() {
            self.playlists = playlists;
        }
        if viewing {
            let (filter, selected) = (std::mem::take(&mut self.filter), self.track_selected);
            if let Ok(tracks) = self.session.library().tracks(playlist_id) {
                self.tracks = tracks;
            }
            self.filter = filter;
            self.apply_filter();
            if let Some(s) = selected.filter(|&s| s < self.visible.len()) {
                self.track_selected = Some(s);
            }
        }
    }
}

/// Fills `out` with indices of `tracks` whose title or artist contains
/// `filter`, case-insensitively (all of them when `filter` is empty).
pub fn filter_indices(tracks: &[Track], filter: &str, out: &mut Vec<u32>) {
    let needle: Vec<char> = filter.chars().flat_map(char::to_lowercase).collect();
    out.clear();
    out.extend(
        tracks
            .iter()
            .enumerate()
            .filter(|(_, t)| {
                needle.is_empty()
                    || contains_ci(&t.title, &needle)
                    || contains_ci(&t.artist, &needle)
            })
            .map(|(i, _)| i as u32),
    );
    out.shrink_to_fit();
}

/// Case-insensitive substring test without allocating (`needle` lowercased).
fn contains_ci(haystack: &str, needle: &[char]) -> bool {
    needle.is_empty()
        || haystack.char_indices().any(|(start, _)| {
            let mut chars = haystack[start..].chars().flat_map(char::to_lowercase);
            needle.iter().all(|n| chars.next() == Some(*n))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn case_insensitive_contains() {
        let needle: Vec<char> = "QUEEN".chars().flat_map(char::to_lowercase).collect();
        assert!(contains_ci("We are Queen fans", &needle));
        assert!(!contains_ci("Que", &needle));
        let cyr: Vec<char> = "ОКЕАН".chars().flat_map(char::to_lowercase).collect();
        assert!(contains_ci("Океан Ельзи", &cyr));
    }
}

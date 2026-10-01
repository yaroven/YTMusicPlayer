//! Key -> [`Action`] mapping. Vim keys and arrows both work.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Quit,
    Help,
    Up,
    Down,
    PageUp,
    PageDown,
    Top,
    Bottom,
    FocusNext,
    Select,
    TogglePause,
    Next,
    Prev,
    SeekBack,
    SeekForward,
    VolumeUp,
    VolumeDown,
    Sync,
    Search,
    FindOnline,
    Shuffle,
    Repeat,
    ToggleQueue,
    PlayNext,
    Like,
    AddToPlaylist,
    SignIn,
}

/// `(keys, description)` rows for the help popup.
pub const HELP: &[(&str, &str)] = &[
    ("↑/↓  k/j", "move"),
    ("PgUp/PgDn  g/G", "page / top / bottom"),
    ("Tab  h/l", "switch pane"),
    ("Enter / double-click", "open playlist / play track"),
    ("Space", "play / pause"),
    ("n / p", "next / previous track"),
    ("← / →  click bar", "seek"),
    ("+ / -", "volume"),
    ("/", "filter tracks (Esc clears)"),
    ("o", "search YouTube Music"),
    ("s / e", "shuffle / repeat (off, all, one)"),
    ("v", "show queue"),
    ("u", "play selected track next"),
    ("f", "like / unlike"),
    ("a", "add to playlist"),
    ("r", "sync library from YouTube"),
    ("L", "sign in with Google (browser)"),
    ("?", "this help"),
    ("q  Ctrl-C", "quit"),
];

pub fn action_for(key: KeyEvent) -> Option<Action> {
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        return (key.code == KeyCode::Char('c')).then_some(Action::Quit);
    }
    Some(match key.code {
        KeyCode::Char('q') => Action::Quit,
        KeyCode::Char('?') => Action::Help,
        KeyCode::Up | KeyCode::Char('k') => Action::Up,
        KeyCode::Down | KeyCode::Char('j') => Action::Down,
        KeyCode::PageUp => Action::PageUp,
        KeyCode::PageDown => Action::PageDown,
        KeyCode::Home | KeyCode::Char('g') => Action::Top,
        KeyCode::End | KeyCode::Char('G') => Action::Bottom,
        KeyCode::Tab | KeyCode::BackTab | KeyCode::Char('h') | KeyCode::Char('l') => {
            Action::FocusNext
        }
        KeyCode::Enter => Action::Select,
        KeyCode::Char(' ') => Action::TogglePause,
        KeyCode::Char('n') => Action::Next,
        KeyCode::Char('p') => Action::Prev,
        KeyCode::Left => Action::SeekBack,
        KeyCode::Right => Action::SeekForward,
        KeyCode::Char('+') | KeyCode::Char('=') => Action::VolumeUp,
        KeyCode::Char('-') => Action::VolumeDown,
        KeyCode::Char('r') => Action::Sync,
        KeyCode::Char('/') => Action::Search,
        KeyCode::Char('o') => Action::FindOnline,
        KeyCode::Char('s') => Action::Shuffle,
        KeyCode::Char('e') => Action::Repeat,
        KeyCode::Char('v') => Action::ToggleQueue,
        KeyCode::Char('u') => Action::PlayNext,
        KeyCode::Char('f') => Action::Like,
        KeyCode::Char('a') => Action::AddToPlaylist,
        KeyCode::Char('L') => Action::SignIn,
        _ => return None,
    })
}

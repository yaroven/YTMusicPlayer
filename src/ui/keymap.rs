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
}

/// `(keys, description)` rows for the help popup.
pub const HELP: &[(&str, &str)] = &[
    ("↑/↓  k/j", "move"),
    ("PgUp/PgDn  g/G", "page / top / bottom"),
    ("Tab  h/l", "switch pane"),
    ("Enter", "open playlist / play track"),
    ("Space", "play / pause"),
    ("n / p", "next / previous track"),
    ("← / →", "seek -5s / +5s"),
    ("+ / -", "volume"),
    ("r", "sync library from YouTube"),
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
        _ => return None,
    })
}

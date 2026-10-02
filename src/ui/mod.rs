//! ratatui rendering. Pure functions of [`App`] state, except that drawing
//! records screen areas for mouse hit-testing and keeps scroll offsets.

pub mod keymap;
pub mod views;

use ratatui::{
    Frame,
    layout::{Constraint, Flex, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::Line,
    widgets::{Block, Clear, List, ListItem, Paragraph, Row, Table},
};

use crate::app::{App, Mode, Prompt};

pub const ACCENT: Color = Color::Red;

pub fn draw(frame: &mut Frame, app: &mut App) {
    let [main, player, status] = Layout::vertical([
        Constraint::Min(5),
        Constraint::Length(3),
        Constraint::Length(1),
    ])
    .areas(frame.area());
    let [sidebar, tracks] =
        Layout::horizontal([Constraint::Length(30), Constraint::Min(20)]).areas(main);

    app.areas.playlists = sidebar;
    app.areas.tracks = tracks;
    views::library::draw_playlists(frame, sidebar, app);
    views::library::draw_tracks(frame, tracks, app);
    views::now_playing::draw(frame, player, app);
    views::now_playing::draw_status(frame, status, app);

    match &mut app.mode {
        Mode::Help => draw_help(frame),
        Mode::AddTo { track, state } => {
            let title = format!(" Add “{}” to ", track.title);
            let items: Vec<ListItem> = app
                .library
                .add_choices()
                .map(|p| ListItem::new(p.title.as_str()))
                .chain(std::iter::once(ListItem::new("New playlist…".italic())))
                .collect();
            let area = centered(frame.area(), 50, items.len() as u16 + 2);
            let list = List::new(items)
                .block(
                    Block::bordered()
                        .title(title)
                        .title_bottom(Line::from(" Enter add · Esc cancel ").right_aligned())
                        .border_style(Style::new().fg(ACCENT)),
                )
                .highlight_style(Style::new().add_modifier(Modifier::REVERSED))
                .highlight_symbol("› ");
            frame.render_widget(Clear, area);
            frame.render_stateful_widget(list, area, state);
        }
        Mode::Menu { entries, state } => {
            let items: Vec<ListItem> = entries.iter().map(|e| ListItem::new(e.label())).collect();
            let area = centered(frame.area(), 40, items.len() as u16 + 2);
            let list = List::new(items)
                .block(
                    Block::bordered()
                        .title(" Playlist ")
                        .border_style(Style::new().fg(ACCENT)),
                )
                .highlight_style(Style::new().add_modifier(Modifier::REVERSED))
                .highlight_symbol("› ");
            frame.render_widget(Clear, area);
            frame.render_stateful_widget(list, area, state);
        }
        Mode::Prompt { purpose, text } => {
            let title = match purpose {
                Prompt::NewPlaylist(_) => " New playlist name ",
                Prompt::Rename(_) => " Rename playlist ",
            };
            let area = centered(frame.area(), 50, 3);
            let input = Paragraph::new(format!("{text}▏")).block(
                Block::bordered()
                    .title(title)
                    .title_bottom(Line::from(" Enter save · Esc cancel ").right_aligned())
                    .border_style(Style::new().fg(ACCENT)),
            );
            frame.render_widget(Clear, area);
            frame.render_widget(input, area);
        }
        Mode::Lyrics => draw_lyrics(frame, app),
        Mode::Normal | Mode::Search | Mode::Find(_) => {}
    }
}

fn centered(outer: Rect, width: u16, height: u16) -> Rect {
    let [area] = Layout::vertical([Constraint::Length(height.min(outer.height))])
        .flex(Flex::Center)
        .areas(outer);
    let [area] = Layout::horizontal([Constraint::Length(width.min(outer.width))])
        .flex(Flex::Center)
        .areas(area);
    area
}

/// Lyrics of the playing track; the current line of synced lyrics is
/// highlighted and kept in the middle.
fn draw_lyrics(frame: &mut Frame, app: &App) {
    let outer = frame.area();
    let area = centered(
        outer,
        outer.width.saturating_sub(8).min(80),
        outer.height.saturating_sub(4),
    );
    let title = app
        .session
        .queue
        .current()
        .map(|t| format!(" {} — {} ", t.title, t.artist))
        .unwrap_or_else(|| " Lyrics ".into());
    let (lines, source, current): (Vec<Line>, String, Option<usize>) = match &app.session.lyrics {
        Some((_, Some(lyrics))) => {
            let current = lyrics.current_line(app.session.player_status.position);
            let lines = lyrics
                .lines
                .iter()
                .enumerate()
                .map(|(i, l)| {
                    let line = Line::from(l.text.clone()).centered();
                    match current {
                        Some(c) if c == i => line.fg(ACCENT).bold(),
                        Some(_) => line.dark_gray(),
                        None => line,
                    }
                })
                .collect();
            (lines, lyrics.source.clone(), current)
        }
        Some((_, None)) => (
            vec![Line::from("No lyrics for this song").centered().dark_gray()],
            String::new(),
            None,
        ),
        None => (
            vec![Line::from("Looking for lyrics…").centered().dark_gray()],
            String::new(),
            None,
        ),
    };
    let inner_height = area.height.saturating_sub(2);
    let scroll = current.map_or(0, |c| (c as u16).saturating_sub(inner_height / 2));
    let block = Block::bordered()
        .title(title)
        .title_bottom(Line::from(format!(" {source} · t / Esc close ")).right_aligned())
        .border_style(Style::new().fg(ACCENT));
    frame.render_widget(Clear, area);
    frame.render_widget(Paragraph::new(lines).block(block).scroll((scroll, 0)), area);
}

fn draw_help(frame: &mut Frame) {
    let area = centered(frame.area(), 58, keymap::HELP.len() as u16 + 2);
    let rows = keymap::HELP
        .iter()
        .map(|(keys, desc)| Row::new([keys.bold(), (*desc).into()]));
    let table = Table::new(rows, [Constraint::Length(22), Constraint::Min(10)]).block(
        Block::bordered()
            .title(" Keys ")
            .title_bottom(Line::from(" any key to close ").right_aligned())
            .border_style(Style::new().fg(ACCENT)),
    );
    frame.render_widget(Clear, area);
    frame.render_widget(table, area);
}

pub use crate::fmt::fmt_time;

pub(crate) fn centered_text(frame: &mut Frame, area: Rect, text: &str) {
    let [line] = Layout::vertical([Constraint::Length(1)])
        .flex(Flex::Center)
        .areas(area);
    frame.render_widget(Paragraph::new(text).centered().dark_gray(), line);
}

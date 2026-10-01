//! ratatui rendering. Pure functions of [`App`] state, except that drawing
//! records screen areas for mouse hit-testing and keeps scroll offsets.

pub mod keymap;
pub mod views;

use std::time::Duration;

use ratatui::{
    Frame,
    layout::{Constraint, Flex, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::Line,
    widgets::{Block, Clear, List, ListItem, Paragraph, Row, Table},
};

use crate::{
    api::models::LIKED_PLAYLIST_ID,
    app::{App, Mode},
};

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
                .playlists
                .iter()
                .filter(|p| p.id != LIKED_PLAYLIST_ID)
                .map(|p| ListItem::new(p.title.as_str()))
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
        Mode::Normal | Mode::Search => {}
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

/// `m:ss` or `h:mm:ss`.
pub fn fmt_time(d: Duration) -> String {
    let s = d.as_secs();
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

pub(crate) fn centered_text(frame: &mut Frame, area: Rect, text: &str) {
    let [line] = Layout::vertical([Constraint::Length(1)])
        .flex(Flex::Center)
        .areas(area);
    frame.render_widget(Paragraph::new(text).centered().dark_gray(), line);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_format() {
        assert_eq!(fmt_time(Duration::from_secs(5)), "0:05");
        assert_eq!(fmt_time(Duration::from_secs(213)), "3:33");
        assert_eq!(fmt_time(Duration::from_secs(3723)), "1:02:03");
    }
}

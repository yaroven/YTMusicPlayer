//! ratatui rendering. Pure functions of [`App`] state; no I/O here.

pub mod keymap;
pub mod views;

use std::time::Duration;

use ratatui::{
    Frame,
    layout::{Constraint, Flex, Layout, Rect},
    style::{Color, Style, Stylize},
    text::Line,
    widgets::{Block, Clear, Paragraph, Row, Table},
};

use crate::app::App;

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

    views::library::draw_playlists(frame, sidebar, app);
    views::library::draw_tracks(frame, tracks, app);
    views::now_playing::draw(frame, player, app);
    views::now_playing::draw_status(frame, status, app);

    if app.show_help {
        draw_help(frame);
    }
}

fn draw_help(frame: &mut Frame) {
    let height = keymap::HELP.len() as u16 + 2;
    let [area] = Layout::vertical([Constraint::Length(height)])
        .flex(Flex::Center)
        .areas(frame.area());
    let [area] = Layout::horizontal([Constraint::Length(50)])
        .flex(Flex::Center)
        .areas(area);
    let rows = keymap::HELP
        .iter()
        .map(|(keys, desc)| Row::new([keys.bold(), (*desc).into()]));
    let table = Table::new(rows, [Constraint::Length(18), Constraint::Min(10)]).block(
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

//! Left: playlists (Liked first). Right: tracks of the selected playlist.

use std::time::Duration;

use ratatui::{
    Frame,
    layout::{Constraint, Rect},
    style::{Modifier, Style, Stylize},
    text::Line,
    widgets::{Block, Cell, List, ListItem, Row, Table},
};

use crate::{
    app::{App, Focus},
    ui::{ACCENT, centered_text, fmt_time},
};

fn pane(title: String, focused: bool) -> Block<'static> {
    let block = Block::bordered().title(Line::from(title).bold());
    if focused {
        block.border_style(Style::new().fg(ACCENT))
    } else {
        block.dark_gray()
    }
}

fn highlight(focused: bool) -> Style {
    if focused {
        Style::new()
            .bg(ACCENT)
            .fg(ratatui::style::Color::White)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::new().add_modifier(Modifier::REVERSED)
    }
}

pub fn draw_playlists(frame: &mut Frame, area: Rect, app: &mut App) {
    let focused = app.focus == Focus::Playlists;
    let block = pane(" Library ".into(), focused);
    if app.playlists.is_empty() {
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let hint = match (app.syncing, app.logged_in) {
            (true, _) => "syncing…",
            (false, true) => "empty — press r to sync",
            (false, false) => "not logged in — run `ytm login`",
        };
        centered_text(frame, inner, hint);
        return;
    }
    let items = app.playlists.iter().map(|p| {
        ListItem::new(Line::from(vec![
            p.title.clone().into(),
            format!(" {}", p.item_count).dark_gray(),
        ]))
    });
    let list = List::new(items)
        .block(block)
        .highlight_style(highlight(focused))
        .highlight_symbol("› ");
    frame.render_stateful_widget(list, area, &mut app.playlist_state);
}

pub fn draw_tracks(frame: &mut Frame, area: Rect, app: &mut App) {
    let focused = app.focus == Focus::Tracks;
    let title = app
        .playlist_state
        .selected()
        .and_then(|i| app.playlists.get(i))
        .map_or(" Tracks ".into(), |p| {
            format!(" {} · {} ", p.title, app.tracks.len())
        });
    let block = pane(title, focused);
    if app.tracks.is_empty() {
        let inner = block.inner(area);
        frame.render_widget(block, area);
        centered_text(frame, inner, "no tracks");
        return;
    }

    let playing_id = app.queue.current().map(|t| t.video_id.as_str());
    let rows = app.tracks.iter().enumerate().map(|(i, t)| {
        let playing = Some(t.video_id.as_str()) == playing_id;
        let marker = if playing {
            "▶".to_owned()
        } else {
            (i + 1).to_string()
        };
        let time = t
            .duration_secs
            .map(|s| fmt_time(Duration::from_secs(s.into())))
            .unwrap_or_default();
        let row = Row::new([
            Cell::from(Line::from(marker).right_aligned()),
            Cell::from(t.title.as_str()),
            Cell::from(t.artist.as_str()),
            Cell::from(Line::from(time).right_aligned()),
        ]);
        if playing { row.fg(ACCENT) } else { row }
    });
    let table = Table::new(
        rows,
        [
            Constraint::Length(4),
            Constraint::Fill(3),
            Constraint::Fill(2),
            Constraint::Length(7),
        ],
    )
    .header(Row::new(["#", "Title", "Artist", "Time"]).dark_gray())
    .block(block)
    .column_spacing(2)
    .row_highlight_style(highlight(focused));
    frame.render_stateful_widget(table, area, &mut app.track_state);
}

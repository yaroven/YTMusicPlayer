//! Left: playlists (Liked first). Right: tracks of the selected playlist, or
//! the play queue. Only rows on screen are turned into widgets.

use std::time::Duration;

use ratatui::{
    Frame,
    layout::{Constraint, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Cell, List, ListItem, Row, Table, TableState},
};

use crate::{
    api::models::Track,
    app::{App, Focus, Mode, TracksView},
    ui::{ACCENT, centered_text, fmt_time},
};

fn pane(title: Line<'static>, focused: bool) -> Block<'static> {
    let block = Block::bordered().title(title.bold());
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
            .fg(Color::White)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::new().add_modifier(Modifier::REVERSED)
    }
}

pub fn draw_playlists(frame: &mut Frame, area: Rect, app: &mut App) {
    let focused = app.focus == Focus::Playlists;
    let block = pane(Line::from(" Library "), focused);
    if app.library.playlists().is_empty() {
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let hint = match (app.session.syncing, app.session.signed_in()) {
            (true, _) => "syncing…",
            (false, true) => "empty — press r to sync",
            (false, false) => "not logged in — run `ytm login`",
        };
        centered_text(frame, inner, hint);
        return;
    }
    let items = app.library.playlists().iter().map(|p| {
        ListItem::new(Line::from(vec![
            Span::raw(p.title.as_str()),
            format!(" {}", p.item_count).dark_gray(),
        ]))
    });
    let list = List::new(items)
        .block(block)
        .highlight_style(highlight(focused))
        .highlight_symbol("› ");
    frame.render_stateful_widget(list, area, &mut app.playlist_state);
}

fn track_row(marker: String, t: &Track, playing: bool, liked: bool) -> Row<'_> {
    let time = t
        .duration_secs
        .map(|s| fmt_time(Duration::from_secs(s.into())))
        .unwrap_or_default();
    let row = Row::new([
        Cell::from(Line::from(marker).right_aligned()),
        Cell::from(if liked {
            Line::from(vec![Span::raw("♥ ").fg(ACCENT), Span::raw(&*t.title)])
        } else {
            Line::from(&*t.title)
        }),
        Cell::from(&*t.artist),
        Cell::from(Line::from(time).right_aligned()),
    ]);
    if playing { row.fg(ACCENT) } else { row }
}

const WIDTHS: [Constraint; 4] = [
    Constraint::Length(5),
    Constraint::Fill(3),
    Constraint::Fill(2),
    Constraint::Length(7),
];

pub fn draw_tracks(frame: &mut Frame, area: Rect, app: &mut App) {
    match app.view {
        TracksView::Playlist => draw_playlist_tracks(frame, area, app),
        TracksView::Queue => draw_queue(frame, area, app),
    }
}

fn draw_playlist_tracks(frame: &mut Frame, area: Rect, app: &mut App) {
    let focused = app.focus == Focus::Tracks;
    let lib = &app.library;
    let name = match lib.source_name() {
        name if name.is_empty() => "Tracks".to_owned(),
        name => name,
    };
    let mut title = vec![Span::raw(format!(" {name} · {} ", lib.len()))];
    let searching = matches!(app.mode, Mode::Search);
    if searching || !lib.filter().is_empty() {
        let cursor = if searching { "▏" } else { "" };
        title.push(Span::raw(format!("/{}{cursor} ", lib.filter())).fg(ACCENT));
    }
    let block = pane(Line::from(title), focused);
    if lib.is_empty() {
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let hint = if lib.showing_results() && lib.filter().is_empty() {
            "nothing found"
        } else if lib.filter().is_empty() {
            "no tracks"
        } else {
            "no matches"
        };
        centered_text(frame, inner, hint);
        return;
    }

    // Keep the selection on screen, then build widgets for that window only.
    let height = area.height.saturating_sub(3) as usize; // borders + header
    let selected = app.library.selected_row().unwrap_or(0);
    if selected < app.track_offset {
        app.track_offset = selected;
    } else if height > 0 && selected >= app.track_offset + height {
        app.track_offset = selected + 1 - height;
    }
    let lib = &app.library;
    let end = (app.track_offset + height).min(lib.len());
    let playing_id = app.session.queue.current().map(|t| t.video_id.clone());
    let rows = (app.track_offset..end).filter_map(|row| {
        let (i, t) = lib.row(row)?;
        let playing = playing_id.as_ref() == Some(&t.video_id);
        let marker = if playing {
            "▶".to_owned()
        } else {
            (i + 1).to_string()
        };
        // Indexed lookup for on-screen rows only.
        Some(track_row(marker, t, playing, lib.is_liked(&t.video_id)))
    });
    let table = Table::new(rows, WIDTHS)
        .header(Row::new(["#", "Title", "Artist", "Time"]).dark_gray())
        .block(block)
        .column_spacing(2)
        .row_highlight_style(highlight(focused));
    let mut state = TableState::default().with_selected(Some(selected - app.track_offset));
    frame.render_stateful_widget(table, area, &mut state);
}

fn draw_queue(frame: &mut Frame, area: Rect, app: &mut App) {
    let focused = app.focus == Focus::Tracks;
    let block = pane(
        Line::from(format!(" Queue · {} ", app.session.queue.len())),
        focused,
    );
    let Some(current) = app.session.queue.current() else {
        let inner = block.inner(area);
        frame.render_widget(block, area);
        centered_text(frame, inner, "queue is empty — play a track");
        return;
    };
    let height = area.height.saturating_sub(3) as usize;
    let rows = std::iter::once(track_row("▶".into(), current, true, false)).chain(
        app.session
            .queue
            .upcoming()
            .take(height.saturating_sub(1))
            .enumerate()
            .map(|(i, t)| track_row((i + 1).to_string(), t, false, false)),
    );
    let table = Table::new(rows, WIDTHS)
        .header(Row::new(["", "Up next", "Artist", "Time"]).dark_gray())
        .block(block)
        .column_spacing(2);
    frame.render_widget(table, area);
}

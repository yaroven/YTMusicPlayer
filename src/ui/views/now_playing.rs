//! Bottom: current track, progress bar, volume; then a one-line status bar.

use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Style, Stylize},
    text::{Line, Span},
    widgets::{Block, LineGauge, Paragraph},
};

use crate::{
    app::App,
    audio::player::PlayState,
    ui::{ACCENT, fmt_time},
};

pub fn draw(frame: &mut Frame, area: Rect, app: &App) {
    let status = &app.player_status;
    let icon = match (app.loading, status.state) {
        (true, _) => "…",
        (false, PlayState::Playing) => "▶",
        (false, PlayState::Paused) => "⏸",
        (false, PlayState::Idle) => "■",
    };
    let title = match app.queue.current() {
        Some(t) => Line::from(vec![
            format!(" {icon} ").fg(ACCENT).bold(),
            t.title.clone().bold(),
            " — ".dark_gray(),
            t.artist.clone().into(),
            " ".into(),
        ]),
        None => Line::from(" ■ nothing playing ".dark_gray()),
    };
    let volume = format!(" vol {:>3}% ", (status.volume * 100.0).round() as u32);
    let queue = match app.queue.position() {
        Some(i) => format!(" {}/{} ", i + 1, app.queue.len()),
        None => String::new(),
    };
    let block = Block::bordered()
        .title(title)
        .title_top(Line::from(volume).right_aligned())
        .title_bottom(Line::from(queue).right_aligned().dark_gray());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let [bar] = Layout::vertical([Constraint::Length(1)]).areas(inner);
    let (label, ratio) = if app.loading {
        ("loading…".to_owned(), 0.0)
    } else {
        match status.duration {
            Some(d) if !d.is_zero() => (
                format!("{} / {}", fmt_time(status.position), fmt_time(d)),
                (status.position.as_secs_f64() / d.as_secs_f64()).clamp(0.0, 1.0),
            ),
            _ => (fmt_time(status.position), 0.0),
        }
    };
    let gauge = LineGauge::default()
        .ratio(ratio)
        .label(label)
        .filled_style(Style::new().fg(ACCENT))
        .unfilled_style(Style::new().dark_gray());
    frame.render_widget(gauge, bar);
}

pub fn draw_status(frame: &mut Frame, area: Rect, app: &App) {
    let hint = Span::from(" ? help · q quit ").dark_gray();
    let [left, right] =
        Layout::horizontal([Constraint::Min(10), Constraint::Length(hint.width() as u16)])
            .areas(area);
    if let Some(status) = &app.status {
        let text = format!(" {}", status.text);
        let line = if status.is_error {
            text.red()
        } else {
            text.into()
        };
        frame.render_widget(Paragraph::new(line), left);
    }
    frame.render_widget(Paragraph::new(hint), right);
}

//! Bottom: current track, progress bar, modes, volume; then a status line.

use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Style, Stylize},
    text::{Line, Span},
    widgets::{Block, LineGauge, Paragraph},
};

use crate::{
    app::{App, Mode},
    audio::{player::PlayState, queue::Repeat},
    ui::{ACCENT, fmt_time},
};

pub fn draw(frame: &mut Frame, area: Rect, app: &mut App) {
    let status = &app.session.player_status;
    let icon = match (app.session.loading, status.state) {
        (true, _) => "…",
        (false, PlayState::Playing) => "▶",
        (false, PlayState::Paused) => "⏸",
        (false, PlayState::Idle) => "■",
    };
    let title = match app.session.queue.current() {
        Some(t) => Line::from(vec![
            format!(" {icon} ").fg(ACCENT).bold(),
            Span::raw(&*t.title).bold(),
            " — ".dark_gray(),
            Span::raw(&*t.artist),
            " ".into(),
        ]),
        None => Line::from(" ■ nothing playing ".dark_gray()),
    };
    let on = |active: bool, text: &'static str| {
        if active {
            Span::from(text).fg(ACCENT)
        } else {
            Span::from(text).dark_gray()
        }
    };
    let repeat = match app.session.queue.repeat {
        Repeat::Off => on(false, "repeat "),
        Repeat::All => on(true, "repeat "),
        Repeat::One => on(true, "repeat1 "),
    };
    let modes = Line::from(vec![
        on(app.session.queue.shuffle, " shuffle "),
        repeat,
        format!("vol {:>3}% ", (status.volume * 100.0).round() as u32).into(),
    ])
    .right_aligned();
    let queue = match app.session.queue.position() {
        Some(i) => format!(" {}/{} ", i + 1, app.session.queue.len()),
        None => String::new(),
    };
    let block = Block::bordered()
        .title(title)
        .title_top(modes)
        .title_bottom(Line::from(queue).right_aligned().dark_gray());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let [bar] = Layout::vertical([Constraint::Length(1)]).areas(inner);
    app.areas.progress = bar;
    let (label, ratio) = if app.session.loading {
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
    let hint_text = match &app.mode {
        Mode::Search => " type to filter · Enter keep · Esc clear ".to_owned(),
        Mode::Find(query) => {
            format!(" search YouTube Music: {query}▏ · Enter search · Esc cancel ")
        }
        _ if app.session.memory.is_empty() => " ? help · q quit ".to_owned(),
        _ => format!(" {} · ? help · q quit ", app.session.memory),
    };
    let hint = Span::from(hint_text).dark_gray();
    let [left, right] =
        Layout::horizontal([Constraint::Min(10), Constraint::Length(hint.width() as u16)])
            .areas(area);
    if let Some(status) = &app.session.status {
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

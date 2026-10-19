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
    session::Sleep,
    tr, trf,
    ui::{ACCENT, fmt_time},
};

pub fn draw(frame: &mut Frame, area: Rect, app: &mut App) {
    let lang = app.session.lang;
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
        None => Line::from(tr!(lang, " ■ nothing playing ", " ■ нічого не грає ").dark_gray()),
    };
    let on = |active: bool, text: &'static str| {
        if active {
            Span::from(text).fg(ACCENT)
        } else {
            Span::from(text).dark_gray()
        }
    };
    let (repeat_label, repeat_one) = (
        tr!(lang, "repeat ", "повтор "),
        tr!(lang, "repeat1 ", "повтор1 "),
    );
    let repeat = match app.session.queue.repeat {
        Repeat::Off => on(false, repeat_label),
        Repeat::All => on(true, repeat_label),
        Repeat::One => on(true, repeat_one),
    };
    let sleep = match app.session.sleep {
        Some(Sleep::EndOfTrack) => {
            tr!(lang, " sleep: end of track ", " сон: кінець пісні ").to_owned()
        }
        Some(Sleep::At(at)) => tr!(
            lang,
            " sleep {}m ",
            " сон {} хв ",
            at.saturating_duration_since(std::time::Instant::now())
                .as_secs()
                / 60
                + 1
        ),
        None => String::new(),
    };
    let modes = Line::from(vec![
        Span::from(sleep).fg(ACCENT),
        on(
            app.session.view().autoplay,
            tr!(lang, " autoplay ", " автоплей "),
        ),
        on(
            app.session.queue.shuffle,
            tr!(lang, " shuffle ", " перемішано "),
        ),
        repeat,
        tr!(
            lang,
            "vol {:>3}% ",
            "гучн. {:>3}% ",
            (status.volume * 100.0).round() as u32
        )
        .into(),
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
        (tr!(lang, "loading…", "завантажую…").to_owned(), 0.0)
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
    let lang = app.session.lang;
    let keys = tr!(lang, "? help · q quit ", "? довідка · q вихід ");
    let hint_text = match &app.mode {
        Mode::Search => tr!(
            lang,
            " type to filter · Enter keep · Esc clear ",
            " друкуй, щоб фільтрувати · Enter лишити · Esc очистити "
        )
        .to_owned(),
        Mode::Find(query) => trf!(
            lang,
            " search YouTube Music: {query}▏ · Enter search · Esc cancel ",
            " пошук у YouTube Music: {query}▏ · Enter шукати · Esc скасувати "
        ),
        _ if app.session.memory.is_empty() => format!(" {keys}"),
        _ => format!(" {} · {keys}", app.session.memory),
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
    } else if app.session.is_offline() {
        // Nothing else to say: keep the reason everything is empty in sight.
        let text = format!(" ⚠ {}", crate::net::offline_text(lang));
        frame.render_widget(Paragraph::new(text.red()), left);
    }
    frame.render_widget(Paragraph::new(hint), right);
}

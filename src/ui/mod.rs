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

use crate::{
    app::{App, Mode, Prompt},
    i18n::Lang,
    tr,
};

pub const ACCENT: Color = Color::Red;

pub fn draw(frame: &mut Frame, app: &mut App) {
    if !app.session.youtube_signed_in() {
        draw_sign_in(frame, app);
        return;
    }
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

    let lang = app.session.lang;
    match &mut app.mode {
        Mode::Help => draw_help(frame, lang),
        Mode::AddTo { track, state } => {
            let title = tr!(lang, " Add “{}” to ", " Додати «{}» до ", track.title);
            let items: Vec<ListItem> = app
                .library
                .add_choices()
                .map(|p| ListItem::new(p.title.as_str()))
                .chain(std::iter::once(ListItem::new(
                    tr!(lang, "New playlist…", "Новий плейлист…").italic(),
                )))
                .collect();
            let area = centered(frame.area(), 50, items.len() as u16 + 2);
            let list = List::new(items)
                .block(
                    Block::bordered()
                        .title(title)
                        .title_bottom(
                            Line::from(tr!(
                                lang,
                                " Enter add · Esc cancel ",
                                " Enter додати · Esc скасувати "
                            ))
                            .right_aligned(),
                        )
                        .border_style(Style::new().fg(ACCENT)),
                )
                .highlight_style(Style::new().add_modifier(Modifier::REVERSED))
                .highlight_symbol("› ");
            frame.render_widget(Clear, area);
            frame.render_stateful_widget(list, area, state);
        }
        Mode::Menu {
            title,
            entries,
            state,
        } => {
            let items: Vec<ListItem> = entries
                .iter()
                .map(|e| ListItem::new(e.label(lang)))
                .collect();
            let area = centered(frame.area(), 40, items.len() as u16 + 2);
            let list = List::new(items)
                .block(
                    Block::bordered()
                        .title(*title)
                        .border_style(Style::new().fg(ACCENT)),
                )
                .highlight_style(Style::new().add_modifier(Modifier::REVERSED))
                .highlight_symbol("› ");
            frame.render_widget(Clear, area);
            frame.render_stateful_widget(list, area, state);
        }
        Mode::Prompt { purpose, text } => {
            let title = match purpose {
                Prompt::NewPlaylist(_) => tr!(lang, " New playlist name ", " Назва плейлиста "),
                Prompt::Rename(_) => tr!(lang, " Rename playlist ", " Перейменувати плейлист "),
                Prompt::SaveQueue => tr!(lang, " Save the queue as ", " Зберегти чергу як "),
            };
            let area = centered(frame.area(), 50, 3);
            let input = Paragraph::new(format!("{text}▏")).block(
                Block::bordered()
                    .title(title)
                    .title_bottom(
                        Line::from(tr!(
                            lang,
                            " Enter save · Esc cancel ",
                            " Enter зберегти · Esc скасувати "
                        ))
                        .right_aligned(),
                    )
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

/// Signed out: nothing but this. The player shows only the account's own
/// music, so it asks for the sign-in first.
fn draw_sign_in(frame: &mut Frame, app: &App) {
    let lang = app.session.lang;
    let [body, status] =
        Layout::vertical([Constraint::Min(5), Constraint::Length(1)]).areas(frame.area());
    let how = if crate::app::sign_in_window() {
        tr!(
            lang,
            "Enter  sign in with Google (opens a window)",
            "Enter  увійти через Google (відкриється вікно)"
        )
        .to_owned()
    } else {
        let browser = crate::audio::extractor::COOKIE_BROWSERS[app.gate_browser];
        tr!(
            lang,
            "Sign in at youtube.com in a browser on this computer, then:\nb  browser: {browser}\nEnter  import its sign-in",
            "Увійди на youtube.com у браузері на цьому комп'ютері, потім:\nb  браузер: {browser}\nEnter  імпортувати його вхід"
        )
        .replace("{browser}", browser)
    };
    let mut lines = vec![
        Line::from("Yarek Music".bold().fg(ACCENT)),
        Line::from(""),
        Line::from(tr!(
            lang,
            "Sign in with your Google account: Yarek Music shows only your own",
            "Увійди зі своїм акаунтом Google: Yarek Music показує лише твою"
        )),
        Line::from(tr!(
            lang,
            "music — your Home and mixes, likes, playlists and albums.",
            "музику — твою Головну й мікси, вподобання, плейлисти й альбоми."
        )),
        Line::from(""),
    ];
    lines.extend(how.lines().map(|l| Line::from(l.to_owned())));
    lines.push(Line::from(tr!(lang, "q  quit", "q  вийти").dim()));
    let height = lines.len() as u16 + 2;
    let area = centered(body, 76, height);
    frame.render_widget(
        Paragraph::new(lines).centered().block(Block::bordered()),
        area,
    );
    views::now_playing::draw_status(frame, status, app);
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
        .unwrap_or_else(|| tr!(app.session.lang, " Lyrics ", " Текст пісні ").into());
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

fn draw_help(frame: &mut Frame, lang: Lang) {
    let help = keymap::help(lang);
    let area = centered(frame.area(), 74, help.len() as u16 + 2);
    let rows = help
        .iter()
        .map(|(keys, desc)| Row::new([keys.bold(), (*desc).into()]));
    let table = Table::new(rows, [Constraint::Length(22), Constraint::Min(10)]).block(
        Block::bordered()
            .title(tr!(lang, " Keys ", " Клавіші "))
            .title_bottom(
                Line::from(tr!(
                    lang,
                    " any key to close ",
                    " будь-яка клавіша закриває "
                ))
                .right_aligned(),
            )
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

//! Terminal UI regression tests: the app runs on a fake player and a seeded
//! library, keys go through the real handler, and the screen is rendered
//! into a text buffer and compared with a stored snapshot
//! (`cargo insta review` after an intended change).

use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{Terminal, backend::TestBackend};

use super::*;
use crate::{
    library_view::LibraryView,
    session::{Changes, SearchResults},
    testing::{Resolves, artist_page, seeded_library, session_with},
};

const WIDTH: u16 = 100;
const HEIGHT: u16 = 30;

async fn app() -> App {
    let library = seeded_library();
    let (session, _player, _events) = session_with(Resolves::Hang, library.clone()).await;
    App::new(session, LibraryView::new(library).unwrap())
}

/// The screen as text, trailing spaces trimmed.
fn screen(app: &mut App) -> String {
    let mut terminal = Terminal::new(TestBackend::new(WIDTH, HEIGHT)).unwrap();
    terminal.draw(|frame| ui::draw(frame, app)).unwrap();
    let buffer = terminal.backend().buffer();
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
                .trim_end()
                .to_owned()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn press(app: &mut App, code: KeyCode) {
    app.on_key(KeyEvent::new(code, KeyModifiers::NONE));
}

fn char(app: &mut App, c: char) {
    press(app, KeyCode::Char(c));
}

#[tokio::test]
async fn library_on_start() {
    let mut app = app().await;
    insta::assert_snapshot!(screen(&mut app));
}

#[tokio::test]
async fn moving_into_the_tracks_and_down() {
    let mut app = app().await;
    press(&mut app, KeyCode::Tab);
    char(&mut app, 'j');
    let s = screen(&mut app);
    assert!(s.contains("Shape of You"), "{s}");
    insta::assert_snapshot!(s);
}

#[tokio::test]
async fn help_lists_the_keys() {
    let mut app = app().await;
    char(&mut app, '?');
    let s = screen(&mut app);
    for key in ["artist's page", "sleep timer", "Chromecast", "lyrics"] {
        assert!(s.contains(key), "help should mention {key}:\n{s}");
    }
    insta::assert_snapshot!(s);
}

#[tokio::test]
async fn playlist_menu() {
    let mut app = app().await;
    char(&mut app, 'm');
    insta::assert_snapshot!(screen(&mut app));
}

#[tokio::test]
async fn artist_page_shows_its_songs_and_back_returns() {
    let mut app = app().await;
    app.session.page = Some(Arc::new(artist_page(12)));
    app.apply(Changes {
        page: true,
        ..Changes::default()
    });
    let s = screen(&mut app);
    assert!(
        s.contains("Test Artist") && s.contains("Hit number 11"),
        "{s}"
    );
    insta::assert_snapshot!(s);

    char(&mut app, 'b');
    let s = screen(&mut app);
    assert!(
        s.contains("Liked music") && s.contains("Bohemian Rhapsody"),
        "{s}"
    );
}

#[tokio::test]
async fn search_results_replace_the_list() {
    let mut app = app().await;
    let page = artist_page(3);
    app.session.search = Some(SearchResults {
        query: "hits".into(),
        kind: crate::catalog::SearchKind::Songs,
        tracks: page.tracks.into(),
        items: Arc::from([]),
    });
    app.apply(Changes {
        search: true,
        ..Changes::default()
    });
    insta::assert_snapshot!(screen(&mut app));
}

#[tokio::test]
async fn sleep_timer_cycles() {
    let mut app = app().await;
    char(&mut app, 'z');
    assert!(app.session.sleep.is_some());
    let s = screen(&mut app);
    assert!(
        s.contains("Sleeping in 15 min") || s.contains("sleep"),
        "{s}"
    );
}

#[tokio::test]
async fn filter_narrows_the_list() {
    let mut app = app().await;
    press(&mut app, KeyCode::Tab);
    char(&mut app, '/');
    for c in "nirv".chars() {
        char(&mut app, c);
    }
    let s = screen(&mut app);
    assert!(
        s.contains("Smells Like Teen Spirit") && !s.contains("Shape of You"),
        "{s}"
    );
}

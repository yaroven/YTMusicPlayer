//! Window regression tests on Slint's headless testing backend: the
//! window is built from a seeded library, snapshots from the core are fed
//! in, callbacks are invoked like clicks, and the test checks what the
//! window shows and which commands it sends to the core.

use std::time::Duration;

use super::*;
use crate::{
    audio::{player::PlayState, queue::Repeat},
    lyrics::{Line, Lyrics},
    session::{Changes, SessionView},
    testing::{artist_page, seeded_library},
};

struct Harness {
    view: Rc<RefCell<View>>,
    ui: MainWindow,
    rx: UnboundedReceiver<Cmd>,
}

fn harness() -> Harness {
    i_slint_backend_testing::init_no_event_loop();
    let library = seeded_library();
    let library_view = Rc::new(RefCell::new(LibraryView::new(library.clone()).unwrap()));
    let (tx, rx) = unbounded_channel();
    let prefs = Prefs {
        cookies: String::new(),
        normalize: true,
        notifications: false,
        tray: true,
        crossfade: 0.0,
    };
    let view = new_view(library, library_view, prefs, tx);
    VIEW.with(|cell| *cell.borrow_mut() = Some(view.clone()));
    let ui = open_window().unwrap();
    Harness { view, ui, rx }
}

impl Harness {
    fn absorb(&self, snap: Snapshot) {
        self.view.borrow_mut().absorb(snap, Some(&self.ui));
    }

    /// Commands sent so far, art downloads left out.
    fn commands(&mut self) -> Vec<Cmd> {
        let mut out = Vec::new();
        while let Ok(cmd) = self.rx.try_recv() {
            if !matches!(cmd, Cmd::FetchArt(..)) {
                out.push(cmd);
            }
        }
        out
    }
}

fn session_view() -> SessionView {
    SessionView {
        now: None,
        loading: false,
        state: PlayState::Idle,
        position: Duration::ZERO,
        duration: None,
        volume: 0.5,
        shuffle: false,
        repeat: Repeat::Off,
        status: None,
        syncing: false,
        searching: false,
        signed_in: true,
        signing_in: false,
        client_id: String::new(),
        memory: String::new(),
        autoplay: true,
        sleep: None,
        casting: None,
        update: None,
        updating: false,
    }
}

fn snapshot(view: SessionView, changes: Changes) -> Snapshot {
    Snapshot {
        view,
        changes,
        search: None,
        page: None,
        home: None,
        lyrics: None,
        related: None,
        queue: None,
        cast_devices: None,
        cast_scanning: false,
        update_done: None,
    }
}

fn track(title: &str) -> Track {
    Track {
        video_id: format!("{title:0>11}").into(),
        title: title.into(),
        artist: "Someone".into(),
        duration_secs: Some(180),
    }
}

#[test]
fn starts_on_the_library() {
    let h = harness();
    assert_eq!(h.ui.get_tracks_title(), "Liked music");
    assert_eq!(h.ui.get_list_mode(), 0);
    assert_eq!(h.ui.get_tracks().row_count(), 3);
    assert_eq!(h.ui.get_playlists().row_count(), 2);
}

#[test]
fn double_click_plays_from_that_row() {
    let mut h = harness();
    h.ui.invoke_play_track(1);
    let cmds = h.commands();
    assert!(
        matches!(&cmds[..], [Cmd::Play(tracks, 1)] if tracks.len() == 3),
        "{} commands",
        cmds.len()
    );
}

#[test]
fn artist_page_lists_five_songs_then_all_of_them() {
    let h = harness();
    let mut snap = snapshot(
        session_view(),
        Changes {
            page: true,
            ..Changes::default()
        },
    );
    snap.page = Some(Arc::new(artist_page(12)));
    h.absorb(snap);
    assert_eq!(h.ui.get_tracks_title(), "Test Artist");
    assert_eq!(h.ui.get_page_kind(), 2, "artist");
    assert!(h.ui.get_page_art_round());
    assert_eq!(h.ui.get_list_mode(), 2, "songs above shelves");
    assert_eq!(h.ui.get_tracks().row_count(), 5);
    assert_eq!(h.ui.get_page_songs(), 12);
    assert_eq!(h.ui.get_shelves().row_count(), 1);

    h.ui.invoke_show_all_songs();
    assert_eq!(h.ui.get_list_mode(), 0, "a plain (virtualized) list");
    assert_eq!(h.ui.get_tracks().row_count(), 12);
    assert!(h.ui.get_can_back());

    h.ui.invoke_go_back();
    assert_eq!(h.ui.get_list_mode(), 2);
    assert_eq!(h.ui.get_tracks().row_count(), 5);
}

#[test]
fn clicking_an_artist_name_opens_their_page() {
    let mut h = harness();
    h.ui.invoke_row_action(1, "artist".into());
    let cmds = h.commands();
    assert!(
        matches!(&cmds[..], [Cmd::OpenArtist(t)] if &*t.artist == "Ed Sheeran"),
        "row 1 is Shape of You"
    );

    // The player bar's artist (row -1) is the playing track's.
    let mut view = session_view();
    view.now = Some(track("Now"));
    h.absorb(snapshot(view, Changes::default()));
    h.ui.invoke_row_action(-1, "artist".into());
    let cmds = h.commands();
    assert!(matches!(&cmds[..], [Cmd::OpenArtist(t)] if &*t.title == "Now"));
}

#[test]
fn an_update_shows_and_installs() {
    let mut h = harness();
    assert_eq!(h.ui.get_update_version(), "");
    h.ui.invoke_check_updates();
    assert!(matches!(&h.commands()[..], [Cmd::CheckUpdates]));

    let mut view = session_view();
    view.update = Some("9.9.9".into());
    h.absorb(snapshot(
        view,
        Changes {
            update: true,
            ..Changes::default()
        },
    ));
    assert_eq!(h.ui.get_update_version(), "9.9.9");
    h.ui.invoke_update_now();
    assert!(matches!(&h.commands()[..], [Cmd::InstallUpdate]));
}

#[test]
fn settings_change_and_are_saved() {
    let mut h = harness();
    h.ui.invoke_setting_toggled("notifications".into());
    assert!(h.ui.get_set_notifications());
    assert!(h.view.borrow().prefs.notifications);
    assert!(matches!(
        &h.commands()[..],
        [Cmd::StoreSetting("notifications", true)]
    ));

    h.ui.invoke_crossfade_changed(4.4);
    assert!(matches!(&h.commands()[..], [Cmd::Crossfade(4)]));
    h.ui.invoke_crossfade_changed(4.2); // same whole second: nothing
    assert!(h.commands().is_empty());

    h.ui.invoke_cookies_cycle();
    let first = crate::audio::extractor::installed_browsers()
        .first()
        .copied()
        .unwrap_or("");
    assert!(matches!(&h.commands()[..], [Cmd::CookiesBrowser(b)] if b == first));
}

#[test]
fn queue_rows_move_and_go() {
    let mut h = harness();
    let mut snap = snapshot(session_view(), Changes::default());
    snap.queue = Some(Arc::from([track("Now"), track("Next"), track("Later")]));
    h.absorb(snap);
    assert_eq!(h.ui.get_queue_rows().row_count(), 3);

    h.ui.invoke_queue_action(2, "up".into());
    h.ui.invoke_queue_action(1, "remove".into());
    h.ui.invoke_queue_action(0, "up".into()); // the playing track: no-op
    let cmds = h.commands();
    assert!(matches!(
        &cmds[..],
        [Cmd::QueueMove(1, 0), Cmd::QueueRemove(0)]
    ));
}

#[test]
fn synced_lyrics_follow_the_song() {
    let h = harness();
    let mut view = session_view();
    view.now = Some(track("Song"));
    view.position = Duration::from_secs(6);
    let mut snap = snapshot(
        view,
        Changes {
            lyrics: true,
            ..Changes::default()
        },
    );
    let line = |s: u64, text: &str| Line {
        at: Some(Duration::from_secs(s)),
        text: text.into(),
    };
    snap.lyrics = Some(Arc::new(Lyrics {
        lines: vec![line(0, "one"), line(5, "two"), line(10, "three")],
        source: "LRCLIB".into(),
    }));
    h.absorb(snap);
    assert_eq!(h.ui.get_lyrics().row_count(), 3);
    assert_eq!(h.ui.get_lyrics_current(), 1, "6 s is the second line");
    let states: Vec<i32> = (0..3)
        .map(|i| h.ui.get_lyrics().row_data(i).unwrap().state)
        .collect();
    assert_eq!(states, [2, 1, 0], "sung, singing, upcoming");
    assert_eq!(h.ui.get_lyrics_source(), "LRCLIB");
    assert!(!h.ui.get_lyrics_loading());
}

#[test]
fn closing_to_the_tray_frees_the_window_and_keeps_the_state() {
    let mut h = harness();
    let mut snap = snapshot(
        session_view(),
        Changes {
            page: true,
            ..Changes::default()
        },
    );
    snap.page = Some(Arc::new(artist_page(7)));
    h.absorb(snap);

    close_window();
    assert!(current_ui().is_none(), "window destroyed");
    assert!(
        h.commands()
            .iter()
            .any(|c| matches!(c, Cmd::Tabs(false, false))),
        "lyrics / related stop"
    );
    assert!(h.view.borrow().art.borrow().thumbs.map.is_empty());

    // Snapshots keep coming while there's no window.
    let mut view = session_view();
    view.now = Some(track("Meanwhile"));
    h.view
        .borrow_mut()
        .absorb(snapshot(view, Changes::default()), None);

    let ui = open_window().unwrap();
    assert_eq!(ui.get_tracks_title(), "Test Artist", "same page");
    assert_eq!(ui.get_now_title(), "Meanwhile", "same player state");
}

#[test]
fn signing_in_closes_the_account_dialog() {
    let h = harness();
    h.ui.set_account_open(true);
    h.absorb(snapshot(
        session_view(),
        Changes {
            account: true,
            ..Changes::default()
        },
    ));
    assert!(!h.ui.get_account_open());
}

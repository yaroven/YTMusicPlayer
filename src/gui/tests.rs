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
        watching: false,
        downloading: 0,
        signed_in: true,
        signing_in: false,
        youtube_signed_in: false,
        youtube_signing_in: false,
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
        explore: None,
        suggestions: None,
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
        explicit: false,
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
fn one_click_plays_from_that_row() {
    let mut h = harness();
    h.ui.invoke_play_track(1);
    // The second click of a double-click: no restart.
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
fn youtube_sign_in_and_out() {
    let mut h = harness();
    assert!(!h.ui.get_yt_signed_in());
    assert_eq!(
        h.ui.get_yt_window(),
        cfg!(any(target_os = "macos", windows))
    );
    h.ui.invoke_yt_sign_in();
    assert!(matches!(&h.commands()[..], [Cmd::YouTubeSignIn(None)]));

    // Import needs a browser picked.
    h.view.borrow_mut().prefs.cookies = String::new();
    h.ui.invoke_yt_import();
    assert!(h.commands().is_empty());
    h.view.borrow_mut().prefs.cookies = "firefox".into();
    h.ui.invoke_yt_import();
    assert!(matches!(&h.commands()[..], [Cmd::YouTubeSignIn(Some(b))] if b == "firefox"));

    let mut view = session_view();
    view.youtube_signed_in = true;
    h.absorb(snapshot(view, Changes::default()));
    assert!(h.ui.get_yt_signed_in());
    h.ui.invoke_yt_sign_out();
    assert!(matches!(&h.commands()[..], [Cmd::YouTubeSignOut]));
}

#[test]
fn explore_loads_and_search_suggests() {
    let mut h = harness();
    h.ui.invoke_nav_to(7);
    assert!(
        matches!(&h.commands()[..], [Cmd::LoadExplore]),
        "Explore asks the catalog for its shelves"
    );
    assert_eq!(h.ui.get_tracks_title(), "Explore");

    let mut snap = snapshot(
        session_view(),
        Changes {
            explore: true,
            suggestions: true,
            ..Changes::default()
        },
    );
    snap.explore = Some(Arc::from([Shelf {
        title: "Charts".into(),
        items: vec![Item {
            kind: ItemKind::Playlist,
            id: "FEmusic_charts".into(),
            title: "Top songs".into(),
            subtitle: "".into(),
            thumbnail: None,
            track: None,
            explicit: false,
        }],
    }]));
    snap.suggestions = Some(Arc::from(["abba dancing queen".to_owned()]));
    h.absorb(snap);
    assert_eq!(h.ui.get_shelves().row_count(), 1);
    assert_eq!(h.ui.get_search_suggestions().row_count(), 1);

    // Typing asks for suggestions, once the typing stops.
    h.ui.invoke_filter_changed("abba".into());
    assert!(h.commands().is_empty(), "debounced");
    i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(400));
    assert!(matches!(&h.commands()[..], [Cmd::Suggest(q)] if q == "abba"));
}

#[test]
fn explicit_songs_are_marked() {
    let h = harness();
    let mut view = session_view();
    view.now = Some(track("Song"));
    let mut snap = snapshot(
        view,
        Changes {
            search: true,
            ..Changes::default()
        },
    );
    let mut explicit = track("Explicit song");
    explicit.explicit = true;
    let tracks: Arc<[Track]> = Arc::from([explicit.clone(), track("Clean song")]);
    snap.search = Some(SearchResults {
        query: "x".into(),
        kind: SearchKind::Songs,
        tracks,
        items: Arc::from([
            Item {
                kind: ItemKind::Song,
                id: explicit.video_id.clone(),
                title: explicit.title.clone(),
                subtitle: explicit.artist.clone(),
                thumbnail: None,
                track: Some(explicit),
                explicit: true,
            },
            Item {
                kind: ItemKind::Song,
                id: "clean".into(),
                title: "Clean song".into(),
                subtitle: "Someone".into(),
                thumbnail: None,
                track: Some(track("Clean song")),
                explicit: false,
            },
        ]),
    });
    h.absorb(snap);
    let rows = h.ui.get_tracks();
    assert_eq!(rows.row_count(), 2);
    assert!(rows.row_data(0).unwrap().explicit, "the E badge shows");
    assert!(!rows.row_data(1).unwrap().explicit);
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

/// The player bar's video button switches the window to the clip (on
/// Linux, which has no system web view, it hands it to the browser).
#[test]
fn the_video_button_switches_to_the_clip() {
    let mut h = harness();
    let mut view = session_view();
    view.now = Some(track("Song"));
    h.absorb(snapshot(view, Changes::default()));
    h.ui.invoke_watch_video();
    let cmds = h.commands();
    #[cfg(any(target_os = "macos", windows))]
    {
        assert!(h.ui.get_expanded(), "the video needs the full-screen view");
        // Opening the web view itself needs a window on screen; here only
        // the command matters (a failure switches back on its own).
        assert!(matches!(cmds.first(), Some(Cmd::VideoOn)));
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    assert!(matches!(&cmds[..], [Cmd::WatchVideo]));
}

#[test]
fn downloads_can_be_stopped_and_deleted() {
    let mut h = harness();
    let mut view = session_view();
    view.downloading = 2;
    h.absorb(snapshot(view, Changes::default()));
    assert_eq!(h.ui.get_downloading(), 2, "the Stop button shows");

    h.ui.invoke_stop_downloads();
    h.ui.invoke_remove_all_downloads();
    let cmds = h.commands();
    assert!(matches!(
        &cmds[..],
        [Cmd::StopDownloads, Cmd::RemoveAllDownloads]
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
        ..Line::default()
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
fn playing_lyrics_light_up_word_by_word() {
    let h = harness();
    let mut view = session_view();
    view.now = Some(track("Song"));
    view.state = PlayState::Playing;
    view.position = Duration::from_millis(100);
    let mut snap = snapshot(
        view,
        Changes {
            lyrics: true,
            ..Changes::default()
        },
    );
    snap.lyrics = Some(Arc::new(Lyrics {
        lines: vec![
            Line {
                at: Some(Duration::ZERO),
                text: "Caught in a landslide no escape from reality".into(),
                ..Line::default()
            },
            Line {
                at: Some(Duration::from_secs(8)),
                text: "Open your eyes".into(),
                ..Line::default()
            },
        ],
        source: "LRCLIB".into(),
    }));
    h.absorb(snap);
    let words = || h.view.borrow().lyrics.progress.map(|(_, w)| w);
    let start = words().unwrap();
    assert!((1..=6).contains(&start), "letters of \"Caught\": {start}");
    // No new snapshot: the window's own timer moves on with the song.
    std::thread::sleep(Duration::from_millis(1500));
    i_slint_backend_testing::mock_elapsed_time(Duration::from_millis(1500));
    assert!(
        words().unwrap() >= 12,
        "letters lit after 1.6 s: {:?}",
        words()
    );
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

/// Renders the real window into `target/*.png` with the software renderer
/// the app ships, so a look at the UI needs no screen:
/// `cargo test -- --ignored --exact gui::tests::screens_png`.
#[test]
#[ignore = "writes target/*.png; needs its own process (a different Slint platform)"]
fn screens_png() {
    use slint::platform::software_renderer::{
        MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType,
    };

    struct Headless(Rc<MinimalSoftwareWindow>, std::time::Instant);
    impl slint::platform::Platform for Headless {
        fn create_window_adapter(
            &self,
        ) -> Result<Rc<dyn slint::platform::WindowAdapter>, slint::PlatformError> {
            Ok(self.0.clone())
        }
        fn duration_since_start(&self) -> Duration {
            self.1.elapsed()
        }
    }
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(Headless(
        window.clone(),
        std::time::Instant::now(),
    )))
    .unwrap();
    let (w, h) = (1280u32, 760u32);
    window.set_size(slint::PhysicalSize::new(w, h));

    let library = seeded_library();
    let library_view = Rc::new(RefCell::new(LibraryView::new(library.clone()).unwrap()));
    let (tx, _rx) = unbounded_channel();
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
    ui.show().unwrap();

    let mut scratch = vec![PremultipliedRgbaColor::default(); (w * h) as usize];
    let mut settle = |frames: usize| {
        for _ in 0..frames {
            slint::platform::update_timers_and_animations();
            std::thread::sleep(Duration::from_millis(25));
            window.request_redraw();
            window.draw_if_needed(|renderer| {
                renderer.render(&mut scratch, w as usize);
            });
        }
    };
    let save = |name: &str| {
        let mut pixels = vec![PremultipliedRgbaColor::default(); (w * h) as usize];
        window.request_redraw();
        window.draw_if_needed(|renderer| {
            renderer.render(&mut pixels, w as usize);
        });
        let raw: Vec<u8> = pixels
            .iter()
            .flat_map(|p| [p.red, p.green, p.blue])
            .collect();
        let out =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("target/{name}.png"));
        image::RgbImage::from_raw(w, h, raw)
            .unwrap()
            .save(&out)
            .unwrap();
        println!("wrote {}", out.display());
    };

    // --- 0. The library list it starts on -----------------------------------
    settle(20);
    save("library");

    // --- 1. Explore: the shelves the catalog sends back ---------------------
    ui.invoke_nav_to(7);
    let mut snap = snapshot(
        session_view(),
        Changes {
            explore: true,
            ..Changes::default()
        },
    );
    let chip = |title: &str, id: &str| Item {
        kind: ItemKind::Playlist,
        id: id.into(),
        title: title.into(),
        subtitle: "".into(),
        thumbnail: None,
        track: None,
        explicit: false,
    };
    let album = |title: &str, by: &str| Item {
        kind: ItemKind::Album,
        id: format!("MPREb_{title}").into(),
        title: title.into(),
        subtitle: format!("Album • {by}").into(),
        thumbnail: None,
        track: None,
        explicit: title.starts_with('T'),
    };
    snap.explore = Some(Arc::from([
        Shelf {
            title: "Browse".into(),
            items: vec![
                chip("New releases", "FEmusic_new_releases"),
                chip("Charts", "FEmusic_charts"),
                chip("Moods & genres", "FEmusic_moods_and_genres"),
            ],
        },
        Shelf {
            title: "New albums & singles".into(),
            items: vec![
                album("Tidal Glass", "Marina Vel"),
                album("Quiet Machines", "Orhan Ray"),
                album("Paper Lanterns", "The Nine Hours"),
                album("Slow Harbour", "Juno Pike"),
            ],
        },
    ]));
    view.borrow_mut().absorb(snap, Some(&ui));
    settle(30);
    save("explore");
    assert_eq!(ui.get_tracks_title(), "Explore");
    assert_eq!(ui.get_shelves().row_count(), 2);

    // --- 2. Search: the "E" badge and the suggestions dropdown --------------
    let mut snap = snapshot(
        session_view(),
        Changes {
            search: true,
            suggestions: true,
            ..Changes::default()
        },
    );
    let song = |title: &str, by: &str, secs: u32, explicit: bool| Track {
        video_id: title.into(),
        title: title.into(),
        artist: by.into(),
        duration_secs: Some(secs),
        explicit,
    };
    // A songs search fills `tracks` and leaves `items` empty (the cards are
    // for the album / artist / playlist tabs), as `Session::search` does.
    snap.search = Some(SearchResults {
        query: "marina".into(),
        kind: SearchKind::Songs,
        tracks: Arc::from([
            song("Tidal Glass", "Marina Vel", 213, true),
            song("Quiet Machines", "Orhan Ray", 194, false),
            song("Paper Lanterns", "The Nine Hours", 246, true),
        ]),
        items: Arc::from([]),
    });
    snap.suggestions = Some(Arc::from([
        "marina vel".to_owned(),
        "marina vel tidal glass".to_owned(),
        "marina vel live".to_owned(),
    ]));
    view.borrow_mut().absorb(snap, Some(&ui));
    settle(10);
    assert_eq!(ui.get_list_mode(), 0, "songs are rows, not cards");
    assert_eq!(ui.get_tracks().row_count(), 3);
    assert_eq!(
        ui.get_tracks().row_data(0).unwrap().time,
        "3:33",
        "rows keep showing their length next to the badge"
    );
    assert!(
        ui.get_tracks().row_data(0).unwrap().explicit
            && !ui.get_tracks().row_data(1).unwrap().explicit,
        "explicit songs are marked"
    );
    // "/" focuses the search box; then type, as a user would (which also
    // filters the shown list).
    window.dispatch_event(slint::platform::WindowEvent::KeyPressed { text: '/'.into() });
    for c in "marina".chars() {
        window.dispatch_event(slint::platform::WindowEvent::KeyPressed { text: c.into() });
    }
    settle(20);
    save("search");
    assert_eq!(ui.get_search_suggestions().row_count(), 3);
    assert_eq!(
        ui.get_tracks().row_count(),
        1,
        "typing filtered the list down to the matching song"
    );

    // --- 3. Now playing: karaoke lyrics -------------------------------------
    let mut session = session_view();
    session.now = Some(track("Tidal Glass"));
    session.position = Duration::from_millis(14_600);
    session.state = PlayState::Paused;
    let mut snap = snapshot(
        session,
        Changes {
            lyrics: true,
            ..Changes::default()
        },
    );
    // Made-up lines: the renderer doesn't care what they say.
    let lrc = "[00:06.00] First line of the test song\n[00:10.00] Second line goes here\n\
        [00:14.00] A longer third line that wraps around the panel nicely\n[00:22.00]\n\
        [00:28.00] After the break the singing starts again\n[00:34.00] And the last line";
    snap.lyrics = Some(Arc::new(Lyrics {
        lines: crate::lyrics::parse_lrc_for_tests(lrc),
        source: "LRCLIB".into(),
    }));
    view.borrow_mut().absorb(snap, Some(&ui));
    ui.set_expanded(true);
    ui.set_now_tab(1);
    settle(40);
    save("lyrics");
    assert!(
        !ui.get_suggest_open() || ui.get_expanded(),
        "the suggestions don't hang over the full-screen view"
    );
    assert_eq!(ui.get_lyrics().row_count(), 7, "6 lines plus the intro");
}

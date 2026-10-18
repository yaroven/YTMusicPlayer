use super::*;
use crate::testing::*;

#[tokio::test]
async fn stops_skipping_after_three_failures_in_a_row() {
    let (mut s, _, _) = session(Resolves::Fail).await;
    s.play(tracks("t", 5), 0);
    settle(&mut s).await;
    assert_eq!(current(&s), "t2", "skipped twice, then stopped");
    let status = s.status.as_ref().unwrap();
    assert!(
        status.is_error && status.text.contains("t2"),
        "{}",
        status.text
    );
    assert!(!s.loading);
}

#[tokio::test]
async fn results_of_a_replaced_selection_are_ignored() {
    let (mut s, _, _) = session(Resolves::Fail).await;
    s.play(tracks("old", 5), 0);
    s.play(tracks("new", 5), 0);
    settle(&mut s).await;
    // Only the new list's failures count: three of them.
    assert_eq!(current(&s), "new2");
}

#[tokio::test]
async fn previous_restarts_the_track_after_three_seconds() {
    let (mut s, player, _) = session(Resolves::Hang).await;
    s.play(tracks("t", 3), 1);
    player.at(Duration::from_secs(5));
    s.refresh_status();
    s.skip(-1);
    assert_eq!(current(&s), "t1");
    assert!(player.calls().contains(&"seek_to 0".to_owned()));

    player.at(Duration::from_secs(1));
    s.refresh_status();
    s.skip(-1);
    assert_eq!(current(&s), "t0");
}

#[tokio::test]
async fn natural_end_advances_repeat_one_replays_stale_ends_ignored() {
    let (mut s, _, events) = session(Resolves::Hang).await;
    s.play(tracks("t", 3), 0);
    events.send(PlayerEvent::Ended { generation: 1 }).unwrap();
    s.next_event().await;
    assert_eq!(current(&s), "t1");

    s.queue.repeat = Repeat::One;
    events.send(PlayerEvent::Ended { generation: 2 }).unwrap();
    s.next_event().await;
    assert_eq!(current(&s), "t1", "repeat one replays");

    events.send(PlayerEvent::Ended { generation: 1 }).unwrap();
    s.next_event().await;
    assert_eq!(current(&s), "t1", "an old track's end changes nothing");
}

#[tokio::test]
async fn restores_saved_volume_and_modes() {
    let library = Arc::new(Library::open_in_memory().unwrap());
    library.set_meta("volume", "0.30").unwrap();
    library.set_meta("repeat", "all").unwrap();
    library.set_meta("shuffle", "1").unwrap();
    let (s, player, _) = session_with(Resolves::Hang, library).await;
    assert!(player.calls().contains(&"volume 0.30".to_owned()));
    assert_eq!(s.queue.repeat, Repeat::All);
    assert!(s.queue.shuffle);
}

#[tokio::test]
async fn view_shows_the_loading_track() {
    let (mut s, _, _) = session(Resolves::Hang).await;
    s.play(tracks("t", 2), 1);
    let view = s.view();
    assert!(view.loading);
    assert_eq!(view.now.unwrap().title.as_ref(), "t1");
    assert!(!view.signed_in);
}

#[tokio::test]
async fn preloaded_track_takes_over_without_reloading() {
    let (mut s, player, events) = session(Resolves::Hang).await;
    s.play(tracks("t", 3), 0);
    // As if maybe_preload had opened the next track as load 9.
    s.preload = Some(9);
    events.send(PlayerEvent::Started { generation: 9 }).unwrap();
    s.next_event().await;
    assert_eq!(current(&s), "t1");
    assert_eq!(s.generation, 9);
    assert!(!player.calls().iter().any(|c| c == "load 9"), "no reload");
}

#[tokio::test]
async fn editing_the_queue_drops_the_preload() {
    let (mut s, player, _) = session(Resolves::Hang).await;
    s.play(tracks("t", 3), 0);
    s.preload = Some(9);
    s.remove_from_queue(0);
    assert!(s.preload.is_none());
    assert!(player.calls().contains(&"cancel_preload".to_owned()));
    assert_eq!(
        s.queue
            .upcoming()
            .map(|t| t.title.to_string())
            .collect::<Vec<_>>(),
        ["t2"]
    );
}

#[tokio::test]
async fn sleep_at_end_of_track_stops_instead_of_advancing() {
    let (mut s, _, events) = session(Resolves::Hang).await;
    s.play(tracks("t", 3), 0);
    s.set_sleep(None);
    events
        .send(PlayerEvent::Ended {
            generation: s.generation,
        })
        .unwrap();
    s.next_event().await;
    assert_eq!(current(&s), "t0", "stayed on the finished track");
    assert!(s.sleep.is_none());
}

#[tokio::test]
async fn plays_are_recorded_in_history() {
    let (mut s, player, _) = session(Resolves::Hang).await;
    s.play(tracks("t", 2), 1);
    player.at(Duration::from_secs(1));
    s.track_started(None);
    let history = s.library().history().unwrap();
    assert_eq!(history[0].title.as_ref(), "t1");
}

/// The youtube.com sign-in alone is enough for likes and playlist edits:
/// they go through YouTube Music's own endpoints, no OAuth client needed.
#[tokio::test]
async fn the_youtube_sign_in_can_edit_the_library() {
    use crate::youtube_session::{Cookies, InMemory, YouTubeSession};

    let (mut s, _, _) = session(Resolves::Fail).await;
    assert!(!s.can_edit(), "no account at all");
    assert!(s.status.as_ref().is_some_and(|st| st.is_error));

    let cookies_file = std::env::temp_dir().join(format!("ytm-edit-test-{}", std::process::id()));
    let youtube = YouTubeSession::load(InMemory::default(), cookies_file.clone());
    youtube
        .sign_in(Cookies::from([
            ("SAPISID".to_owned(), "secret".to_owned()),
            ("SID".to_owned(), "secret".to_owned()),
        ]))
        .unwrap();
    s.deps.youtube = youtube;
    assert!(s.can_edit());
    std::fs::remove_file(cookies_file).ok();
}

/// Downloads can be stopped while they run, and the lot deleted afterwards.
#[tokio::test]
async fn downloads_stop_and_can_be_deleted() {
    let (mut s, _, _) = session(Resolves::Fail).await;
    s.download(tracks("d", 3).to_vec());
    assert_eq!(s.downloading.len(), 3, "queued for download");

    s.cancel_downloads();
    assert!(s.downloading.is_empty(), "nothing left to download");
    settle(&mut s).await;
    let status = s.status.as_ref().unwrap();
    assert!(
        !status.is_error && status.text.contains("Stopped"),
        "{}",
        status.text
    );

    // Nothing was downloaded, so deleting them all is a no-op that still
    // reports itself.
    s.remove_all_downloads();
    settle(&mut s).await;
    let status = s.status.as_ref().unwrap();
    assert!(status.text.starts_with("Deleted 0"), "{}", status.text);
}

/// Watching the clip of nothing says so instead of opening a window.
#[tokio::test]
async fn watching_needs_a_playing_song() {
    let (mut s, _, _) = session(Resolves::Fail).await;
    s.watch_video();
    assert!(!s.watching);
    let status = s.status.as_ref().unwrap();
    assert!(status.is_error && status.text.contains("Nothing is playing"));
}

/// Moving a song in a playlist that was never synced moves it here (there
/// is no YouTube id for the entry yet) and says so.
#[tokio::test]
async fn songs_move_in_a_playlist_and_history_entries_go() {
    use crate::{api::models::Playlist, storage::SyncedPlaylist};

    let library = Arc::new(crate::storage::Library::open_in_memory().unwrap());
    let playlist = Playlist {
        id: "PLmine".into(),
        title: "Mine".into(),
        item_count: 3,
        etag: None,
    };
    library
        .replace_synced(&[SyncedPlaylist {
            playlist: playlist.clone(),
            tracks: tracks("m", 3),
            item_ids: Vec::new(),
        }])
        .unwrap();
    let (mut s, _, _) = session_with(Resolves::Fail, library.clone()).await;

    s.move_in_playlist(playlist, 2, 0);
    let ids: Vec<_> = library
        .tracks("PLmine")
        .unwrap()
        .iter()
        .map(|t| t.video_id.to_string())
        .collect();
    assert_eq!(ids, ["m0000000002", "m0000000000", "m0000000001"]);
    let changes = s.next_event().await;
    assert_eq!(changes.playlist.as_deref(), Some("PLmine"));

    library.add_history(&tracks("h", 2)[0]).unwrap();
    library.add_history(&tracks("h", 2)[1]).unwrap();
    s.remove_from_history("h0000000000".into());
    let changes = s.next_event().await;
    assert!(changes.history);
    assert_eq!(library.history().unwrap().len(), 1);
    s.clear_history();
    assert!(s.next_event().await.history);
    assert!(library.history().unwrap().is_empty());
}

/// Saving the queue needs an account; without one it says so.
#[tokio::test]
async fn saving_the_queue_needs_an_account() {
    let (mut s, _, _) = session(Resolves::Hang).await;
    s.play(tracks("q", 3), 0);
    s.save_queue_as_playlist("Road mix");
    let status = s.status.as_ref().unwrap();
    assert!(status.is_error, "{}", status.text);
}

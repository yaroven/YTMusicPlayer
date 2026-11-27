//! The window's callbacks: every click and key the Slint markup reports,
//! turned into a [`LibraryView`] step on the UI thread or a [`Cmd`] for
//! the core.

use super::{
    models::{LAST_PLAY, SUGGEST_DELAY, SUGGEST_TIMER},
    *,
};

pub(super) fn wire_callbacks(
    ui: &MainWindow,
    view: &Rc<RefCell<View>>,
    cmd_tx: &UnboundedSender<Cmd>,
    library: &Arc<Library>,
) {
    let send = |cmd: Cmd, tx: &UnboundedSender<Cmd>| {
        let _ = tx.send(cmd);
    };
    // Each handler gets `tx`, `view` and `ui` (names passed in for hygiene).
    macro_rules! on {
        // `||` is a single token, so zero-argument closures need their own arm.
        ($setter:ident, [$tx:ident, $view:ident, $ui:ident], || $body:expr) => {
            on!($setter, [$tx, $view, $ui], | | $body)
        };
        ($setter:ident, [$tx:ident, $view:ident, $ui:ident], |$($arg:ident),*| $body:expr) => {{
            let ($tx, $view, weak) = (cmd_tx.clone(), view.clone(), ui.as_weak());
            ui.$setter(move |$($arg),*| {
                let Some($ui) = weak.upgrade() else { return };
                #[allow(unused_variables)]
                let ($tx, $view, $ui) = (&$tx, &$view, &$ui);
                $body
            });
        }};
    }

    // Starting playback from the list remembers where the queue came from.
    let play =
        |ui: &MainWindow, view: &RefCell<View>, cmd: Option<Cmd>, tx: &UnboundedSender<Cmd>| {
            if let Some(cmd) = cmd {
                let source: SharedString = view.borrow().library.borrow().source_name().into();
                ui.set_queue_source(source.clone());
                *view.borrow().queue_source.borrow_mut() = source;
                let _ = tx.send(cmd);
            }
        };
    // Runs a navigation step on the library view, then mirrors it.
    let navigate = |ui: &MainWindow,
                    view: &RefCell<View>,
                    step: &dyn Fn(&mut LibraryView, &View) -> Result<()>| {
        let mut v = view.borrow_mut();
        let result = {
            let lib = v.library.clone();
            let mut lib = lib.borrow_mut();
            step(&mut lib, &v)
        };
        if let Err(err) = result {
            tracing::warn!(%err, "loading the list");
            ui.set_status_text(format!("library: {err:#}").into());
            ui.set_status_error(true);
        }
        v.sync_list(ui);
    };

    on!(on_select_playlist, [tx, view, ui], |index| {
        if let Ok(i) = usize::try_from(index) {
            navigate(ui, view, &|lib, _| lib.select_playlist(Some(i)));
        }
    });
    on!(on_nav_to, [tx, view, ui], |nav| {
        if nav == 1 {
            send(Cmd::LoadHome, tx);
        }
        if nav == 7 {
            send(Cmd::LoadExplore, tx);
        }
        navigate(ui, view, &|lib, v| match nav {
            1 => lib.show_home(v.home.as_deref()),
            7 => {
                lib.show_explore(v.explore.as_deref());
                Ok(())
            }
            2 => lib.show_history(),
            3 => lib.show_downloads(),
            4 => lib.show_saved(ItemKind::Album),
            5 => lib.show_saved(ItemKind::Artist),
            _ => {
                lib.show_results();
                Ok(())
            }
        });
    });
    on!(on_go_back, [tx, view, ui], || navigate(
        ui,
        view,
        &|lib, _| {
            lib.back();
            Ok(())
        }
    ));
    on!(on_select_track, [tx, view, ui], |row| {
        ui.set_selected_track(row);
        view.borrow().selection_from(ui);
    });
    on!(on_lyrics_seek, [tx, view, ui], |ms| {
        if let Ok(ms) = u64::try_from(ms) {
            send(Cmd::SeekTo(Duration::from_millis(ms)), tx);
        }
    });
    on!(on_play_track, [tx, view, ui], |row| {
        // One click plays; the second click of a double-click doesn't
        // start the song over.
        let now = std::time::Instant::now();
        let repeat = LAST_PLAY.replace(Some((row, now))).is_some_and(|(r, at)| {
            r == row && now.duration_since(at) < std::time::Duration::from_millis(500)
        });
        if repeat {
            return;
        }
        ui.set_selected_track(row);
        let cmd = {
            let v = view.borrow();
            v.selection_from(ui);
            let lib = v.library.borrow();
            lib.play_from(lib.selected_row())
                .map(|(tracks, index)| Cmd::Play(tracks, index))
        };
        play(ui, view, cmd, tx);
    });
    on!(on_play_all, [tx, view, ui], || {
        let cmd = view
            .borrow()
            .library
            .borrow()
            .play_from(None)
            .map(|(tracks, index)| Cmd::Play(tracks, index));
        play(ui, view, cmd, tx);
    });
    on!(on_shuffle_play, [tx, view, ui], || {
        let cmd = view
            .borrow()
            .library
            .borrow()
            .all_tracks()
            .map(Cmd::ShufflePlay);
        play(ui, view, cmd, tx);
    });
    on!(on_start_radio, [tx, view, ui], || {
        // The selected song's radio, else the list's first song's.
        let v = view.borrow();
        v.selection_from(ui);
        let lib = v.library.borrow();
        let seed = lib
            .selected_track()
            .cloned()
            .or_else(|| lib.row(0).map(|(_, t)| t.clone()));
        if let Some(track) = seed {
            let source: SharedString = format!("{} radio", track.title).into();
            ui.set_queue_source(source.clone());
            *v.queue_source.borrow_mut() = source;
            send(Cmd::Radio(track), tx);
        }
    });
    on!(on_jump, [tx, view, ui], |row| {
        // Row 0 is the playing track itself.
        if let Some(n) = usize::try_from(row).ok().and_then(|r| r.checked_sub(1)) {
            send(Cmd::JumpTo(n), tx);
        }
    });
    on!(on_filter_changed, [tx, view, ui], |text| {
        let v = view.borrow();
        v.library.borrow_mut().set_filter(text.as_str());
        ui.set_selected_track(-1);
        v.tracks.reset();
        // One request after the typing stops, not one per keystroke.
        let (query, tx) = (text.to_string(), tx.clone());
        SUGGEST_TIMER.with(|timer| {
            timer.start(slint::TimerMode::SingleShot, SUGGEST_DELAY, move || {
                send(Cmd::Suggest(query.clone()), &tx)
            });
        });
    });
    on!(on_toggle_pause, [tx, view, ui], || send(
        Cmd::TogglePause,
        tx
    ));
    on!(on_next, [tx, view, ui], || send(Cmd::Next, tx));
    on!(on_prev, [tx, view, ui], || send(Cmd::Prev, tx));
    on!(on_seek, [tx, view, ui], |ratio| send(
        Cmd::SeekRatio(ratio.into()),
        tx
    ));
    on!(on_volume_changed, [tx, view, ui], |v| send(
        Cmd::Volume(v),
        tx
    ));
    on!(on_toggle_shuffle, [tx, view, ui], || send(Cmd::Shuffle, tx));
    on!(on_cycle_repeat, [tx, view, ui], || send(Cmd::Repeat, tx));
    on!(on_sync, [tx, view, ui], || send(Cmd::Sync, tx));
    on!(on_toggle_like, [tx, view, ui], || {
        // The player bar's heart (and `f`) belongs to the playing track;
        // rows have their own hearts.
        let v = view.borrow();
        if let Some(track) = v.now.clone().or_else(|| v.target(ui)) {
            send(Cmd::Like(track), tx);
        }
    });
    on!(on_dislike_now, [tx, view, ui], || {
        if let Some(track) = view.borrow().now.clone() {
            send(Cmd::Dislike(track), tx);
        }
    });
    on!(on_play_next, [tx, view, ui], || {
        if let Some(track) = view.borrow().target(ui) {
            send(Cmd::PlayNext(track), tx);
        }
    });
    on!(on_search_online, [tx, view, ui], |text| {
        let query = text.trim();
        if !query.is_empty() {
            // A new query keeps the category being looked at.
            let kind = {
                let v = view.borrow();
                let lib = v.library.borrow();
                match lib.source() {
                    Source::Search => lib.search_kind().unwrap_or(SearchKind::Songs),
                    _ => SearchKind::Songs,
                }
            };
            send(Cmd::Search(query.to_owned(), kind), tx);
        }
    });
    on!(on_search_category, [tx, view, ui], |k| {
        let query = view
            .borrow()
            .library
            .borrow()
            .search_query()
            .map(str::to_owned);
        let kind = usize::try_from(k)
            .ok()
            .and_then(|k| SearchKind::ALL.get(k).copied());
        if let (Some(query), Some(kind)) = (query, kind) {
            send(Cmd::Search(query, kind), tx);
        }
    });
    on!(on_select_download_list, [tx, view, ui], |i| {
        let list = usize::try_from(i).ok();
        let mut v = view.borrow_mut();
        if let Err(err) = v.library.borrow_mut().select_download_list(list) {
            tracing::warn!(%err, "downloaded playlist");
        }
        v.sync_list(ui);
    });
    on!(on_like_row, [tx, view, ui], |row| {
        if let Some(track) = usize::try_from(row)
            .ok()
            .and_then(|r| view.borrow().tracks.track(r))
        {
            send(Cmd::Like(track), tx);
        }
    });
    on!(on_row_action, [tx, view, ui], |row, action| {
        let mut v = view.borrow_mut();
        let Some(track) = v.row_track(row) else {
            return;
        };
        let cmd = match action.as_str() {
            "next" => Cmd::PlayNext(track),
            "queue" => Cmd::AddToQueue(vec![track]),
            "radio" => {
                let source: SharedString = format!("{} radio", track.title).into();
                ui.set_queue_source(source.clone());
                *v.queue_source.borrow_mut() = source;
                Cmd::Radio(track)
            }
            "dislike" => Cmd::Dislike(track),
            "artist" => Cmd::OpenArtist(track),
            "download" if v.library.borrow().is_downloaded(&track.video_id) => {
                Cmd::RemoveDownload(track.video_id)
            }
            "download" => Cmd::Download(vec![track]),
            "remove" => {
                let lib = v.library.borrow();
                match (lib.source(), lib.shown_playlist()) {
                    (Source::Downloads, _) => Cmd::RemoveDownload(track.video_id),
                    (Source::History, _) => Cmd::RemoveFromHistory(track.video_id),
                    (_, Some(p)) if row >= 0 => Cmd::RemoveFrom(p.clone(), track),
                    _ => return,
                }
            }
            "up" | "down" => {
                let lib = v.library.borrow();
                let (Some(playlist), Ok(r)) = (lib.shown_playlist(), usize::try_from(row)) else {
                    return;
                };
                let Some((from, _)) = lib.row(r) else { return };
                let to = if action == "up" {
                    from.checked_sub(1)
                } else {
                    Some(from + 1).filter(|&t| t < lib.total())
                };
                let Some(to) = to else { return };
                Cmd::MoveInPlaylist(playlist.clone(), from, to)
            }
            "add" => {
                // The save-to popup acts on the selection (or the playing track).
                ui.set_selected_track(row.max(-1));
                v.selection_from(ui);
                v.pending_add = None;
                return;
            }
            _ => return,
        };
        send(cmd, tx);
    });
    // A card opens its page: the card's title and art show at once
    // (`show_preview`), the songs when the page arrives.
    let open = move |ui: &MainWindow,
                     view: &RefCell<View>,
                     tx: &UnboundedSender<Cmd>,
                     item: Option<Item>| {
        let Some(item) = item else { return };
        let Some(cmd) = View::open_item(&item) else {
            return;
        };
        if matches!(cmd, Cmd::OpenPage(_)) {
            navigate(ui, view, &|lib, _| {
                lib.show_preview(&item);
                Ok(())
            });
        }
        send(cmd, tx);
    };
    // Mouse down on a card: its page starts loading before the click ends.
    let prefetch = move |tx: &UnboundedSender<Cmd>, item: Option<Item>| {
        if let Some(item) = item.filter(|i| i.track.is_none() && i.kind != ItemKind::Song) {
            send(Cmd::Prefetch(item.id.to_string()), tx);
        }
    };
    let card = |view: &RefCell<View>, i: i32| {
        let v = view.borrow();
        let lib = v.library.borrow();
        usize::try_from(i)
            .ok()
            .and_then(|i| lib.items().get(i).cloned())
    };
    let shelf_card = |view: &RefCell<View>, s: i32, c: i32| {
        let v = view.borrow();
        let lib = v.library.borrow();
        usize::try_from(s)
            .ok()
            .zip(usize::try_from(c).ok())
            .and_then(|(s, c)| lib.shelves().get(s)?.items.get(c).cloned())
    };
    let related_card = |view: &RefCell<View>, s: i32, c: i32| {
        let v = view.borrow();
        usize::try_from(s)
            .ok()
            .zip(usize::try_from(c).ok())
            .and_then(|(s, c)| v.related_data.get(s)?.items.get(c).cloned())
    };
    on!(on_open_card, [tx, view, ui], |i| open(
        ui,
        view,
        tx,
        card(view, i)
    ));
    on!(on_open_shelf_card, [tx, view, ui], |s, c| open(
        ui,
        view,
        tx,
        shelf_card(view, s, c)
    ));
    on!(on_open_related, [tx, view, ui], |s, c| {
        let item = related_card(view, s, c);
        if item.as_ref().is_some_and(|i| i.track.is_none()) {
            ui.set_expanded(false);
        }
        open(ui, view, tx, item);
    });
    on!(on_prefetch_card, [tx, view, ui], |i| prefetch(
        tx,
        card(view, i)
    ));
    on!(on_prefetch_shelf_card, [tx, view, ui], |s, c| prefetch(
        tx,
        shelf_card(view, s, c)
    ));
    on!(on_prefetch_related, [tx, view, ui], |s, c| prefetch(
        tx,
        related_card(view, s, c)
    ));
    on!(on_toggle_saved, [tx, view, ui], || {
        let item = view.borrow().library.borrow().page_item();
        if let Some((item, channel)) = item {
            send(Cmd::ToggleSaved(item, channel), tx);
        }
    });
    on!(on_save_queue_as, [tx, view, ui], |name| {
        send(Cmd::SaveQueue(name.to_string()), tx)
    });
    on!(on_clear_history, [tx, view, ui], || send(
        Cmd::ClearHistory,
        tx
    ));
    on!(on_retry, [tx, view, ui], || send(Cmd::Retry, tx));
    on!(on_select_home_chip, [tx, view, ui], |i| {
        let v = view.borrow();
        let params = usize::try_from(i)
            .ok()
            .and_then(|i| v.home_chips.get(i))
            .map(|c| c.params.clone());
        send(Cmd::HomeChip(params), tx);
    });
    on!(on_more_home, [tx, view, ui], || send(Cmd::MoreHome, tx));
    on!(on_cycle_sort, [tx, view, ui], || {
        let mut v = view.borrow_mut();
        {
            let mut lib = v.library.borrow_mut();
            let next = lib.sort().next();
            lib.set_sort(next);
        }
        v.sync_list(ui);
    });
    on!(on_set_language, [tx, view, ui], |code| {
        let lang = Lang::from_code(&code);
        {
            let mut v = view.borrow_mut();
            v.lang = lang;
            v.library.borrow_mut().set_language(lang);
        }
        apply_language(ui, lang);
        // The shown list's own title ("History", "4 songs") is rebuilt the
        // next time it is shown; the sidebar and the rest change now.
        view.borrow_mut().sync_list(ui);
        send(Cmd::SetLanguage(code.into()), tx);
    });
    on!(on_watch_video, [tx, view, ui], || {
        #[cfg(any(target_os = "macos", windows))]
        {
            let mut v = view.borrow_mut();
            if let Some(video) = v.video.take() {
                ui.set_video_on(false);
                send(Cmd::VideoOff(video.position()), tx);
                return;
            }
            if v.last.as_ref().and_then(|s| s.now.as_ref()).is_none() {
                return;
            }
            drop(v);
            // The video needs the big area: open it, and the rectangle's
            // own callback starts the clip once it has a place on screen.
            ui.set_expanded(true);
            ui.set_video_on(true);
            send(Cmd::VideoOn, tx);
        }
        #[cfg(not(any(target_os = "macos", windows)))]
        send(Cmd::WatchVideo, tx);
    });
    #[cfg(any(target_os = "macos", windows))]
    on!(on_video_rect, [tx, view, ui], |x, y, w, h| {
        let mut v = view.borrow_mut();
        v.video_rect = (x, y, w, h);
        if let Some(video) = &v.video {
            return video.set_bounds(v.video_rect);
        }
        if !ui.get_video_on() {
            return;
        }
        // The song's clip, where the song is (the clip's intro added).
        let Some((id, at)) = v.last.as_ref().and_then(|s| {
            let (clip, offset) = s.video.clone()?;
            Some((clip, s.position + Duration::from_secs_f64(offset)))
        }) else {
            return;
        };
        match video::VideoView::open(ui, id, at, v.video_rect) {
            Ok(video) => v.video = Some(video),
            Err(err) => {
                tracing::warn!("opening the video: {err:#}");
                ui.set_video_on(false);
                send(Cmd::VideoOff(None), tx);
            }
        }
    });
    #[cfg(any(target_os = "macos", windows))]
    on!(on_video_off, [tx, view, ui], || {
        let at = view.borrow_mut().video.take().and_then(|v| v.position());
        send(Cmd::VideoOff(at), tx);
    });
    #[cfg(not(any(target_os = "macos", windows)))]
    on!(on_video_rect, [tx, view, ui], |_x, _y, _w, _h| {});
    #[cfg(not(any(target_os = "macos", windows)))]
    on!(on_video_off, [tx, view, ui], || {});
    on!(on_stop_downloads, [tx, view, ui], || send(
        Cmd::StopDownloads,
        tx
    ));
    on!(on_remove_all_downloads, [tx, view, ui], || send(
        Cmd::RemoveAllDownloads,
        tx
    ));
    on!(on_download_all, [tx, view, ui], || {
        let tracks = view.borrow().library.borrow().all_tracks();
        if let Some(tracks) = tracks {
            send(Cmd::Download(tracks.to_vec()), tx);
        }
    });
    on!(on_new_playlist, [tx, view, ui], |name| {
        let name = name.trim();
        if !name.is_empty() {
            let track = view.borrow_mut().pending_add.take();
            send(Cmd::CreatePlaylist(name.to_owned(), track), tx);
        }
    });
    on!(on_rename_playlist, [tx, view, ui], |name| {
        let name = name.trim();
        let playlist = view.borrow().library.borrow().shown_playlist().cloned();
        if let (false, Some(p)) = (name.is_empty(), playlist) {
            send(Cmd::RenamePlaylist(p, name.to_owned()), tx);
        }
    });
    on!(on_delete_playlist, [tx, view, ui], || {
        let playlist = view.borrow().library.borrow().shown_playlist().cloned();
        if let Some(p) = playlist {
            send(Cmd::DeletePlaylist(p), tx);
        }
    });
    on!(on_queue_action, [tx, view, ui], |row, action| {
        if action.as_str() == "artist" {
            let track = usize::try_from(row)
                .ok()
                .and_then(|r| view.borrow().queue.track(r));
            if let Some(track) = track {
                ui.set_expanded(false);
                send(Cmd::OpenArtist(track), tx);
            }
            return;
        }
        // Row 0 is the playing track; upcoming tracks count from row 1.
        let Some(n) = usize::try_from(row).ok().and_then(|r| r.checked_sub(1)) else {
            return;
        };
        let cmd = match action.as_str() {
            "up" if n > 0 => Cmd::QueueMove(n, n - 1),
            "down" => Cmd::QueueMove(n, n + 1),
            "remove" => Cmd::QueueRemove(n),
            _ => return,
        };
        send(cmd, tx);
    });
    on!(on_clear_queue, [tx, view, ui], || send(Cmd::ClearQueue, tx));
    on!(on_toggle_autoplay, [tx, view, ui], || send(
        Cmd::ToggleAutoplay,
        tx
    ));
    on!(on_set_sleep, [tx, view, ui], |minutes| send(
        Cmd::Sleep(minutes),
        tx
    ));
    on!(on_cast_scan, [tx, view, ui], || send(Cmd::CastScan, tx));
    on!(on_cast_to, [tx, view, ui], |i| send(
        Cmd::CastTo(usize::try_from(i).ok()),
        tx
    ));
    on!(on_now_tab_changed, [tx, view, ui], |tab| send(
        Cmd::Tabs(tab == 1, tab == 2),
        tx
    ));
    on!(on_setting_toggled, [tx, view, ui], |name| {
        match name.as_str() {
            "normalize_volume" => {
                let on = !ui.get_set_normalize();
                ui.set_set_normalize(on);
                view.borrow_mut().prefs.normalize = on;
                send(Cmd::Normalize(on), tx);
            }
            "notifications" => {
                let on = !ui.get_set_notifications();
                ui.set_set_notifications(on);
                view.borrow_mut().prefs.notifications = on;
                send(Cmd::StoreSetting("notifications", on), tx);
            }
            "tray" => {
                let on = !ui.get_set_tray();
                ui.set_set_tray(on);
                view.borrow_mut().prefs.tray = on;
                send(Cmd::StoreSetting("tray", on), tx);
            }
            _ => {}
        }
    });
    on!(on_crossfade_changed, [tx, view, ui], |secs| {
        // The slider reports every move: act on whole seconds only.
        let secs = secs.round().clamp(0.0, 12.0);
        if secs != ui.get_set_crossfade() {
            ui.set_set_crossfade(secs);
            view.borrow_mut().prefs.crossfade = secs;
            send(Cmd::Crossfade(secs as u64), tx);
        }
    });
    on!(on_update_now, [tx, view, ui], || send(
        Cmd::InstallUpdate,
        tx
    ));
    on!(on_check_updates, [tx, view, ui], || send(
        Cmd::CheckUpdates,
        tx
    ));
    on!(on_show_all_songs, [tx, view, ui], || navigate(
        ui,
        view,
        &|lib, _| {
            lib.show_songs();
            Ok(())
        }
    ));
    on!(on_cookies_cycle, [tx, view, ui], || {
        // Off, then the browsers that have a profile on this computer.
        let choices: Vec<&str> = std::iter::once("")
            .chain(crate::signin::installed_browsers())
            .collect();
        let mut v = view.borrow_mut();
        let at = choices.iter().position(|c| *c == v.prefs.cookies);
        let next = choices[at.map_or(0, |i| (i + 1) % choices.len())];
        v.prefs.cookies = next.to_owned();
        ui.set_set_cookies(cookies_label(next).into());
        send(Cmd::CookiesBrowser(next.to_owned()), tx);
    });
    let library = library.clone();
    on!(on_sidebar_resized, [tx, view, ui], |width| {
        if let Err(err) = library.set_meta("sidebar_width", &format!("{width:.0}")) {
            tracing::warn!(%err, "saving sidebar width");
        }
    });
    on!(on_account_opened, [tx, view, ui], || ui
        .set_download_file(downloaded_client_name().into()));
    on!(on_save_client, [tx, view, ui], |id, secret| send(
        Cmd::SaveClient(id.trim().to_owned(), secret.trim().to_owned()),
        tx
    ));
    on!(on_import_downloaded, [tx, view, ui], || {
        if let Some(path) = account::downloaded_client_json() {
            send(Cmd::ImportClient(path), tx);
        }
    });
    on!(on_sign_in, [tx, view, ui], || send(Cmd::SignIn, tx));
    on!(on_yt_sign_in, [tx, view, ui], || send(
        Cmd::YouTubeSignIn(None),
        tx
    ));
    on!(on_yt_sign_out, [tx, view, ui], || send(
        Cmd::YouTubeSignOut,
        tx
    ));
    on!(on_yt_import, [tx, view, ui], || {
        let browser = view.borrow().prefs.cookies.clone();
        if crate::signin::COOKIE_BROWSERS.contains(&browser.as_str()) {
            send(Cmd::YouTubeSignIn(Some(browser)), tx);
        } else {
            ui.set_status_text("Pick the browser you're signed in to YouTube with first".into());
        }
    });
    on!(on_sign_out, [tx, view, ui], || send(Cmd::SignOut, tx));
    on!(on_open_console, [tx, view, ui], || {
        let _ = open::that_detached(account::CONSOLE_URL);
    });
    on!(on_add_to, [tx, view, ui], |index| {
        let mut v = view.borrow_mut();
        match index {
            // "New playlist…" from the save-to menu: create, then add.
            -1 => v.pending_add = v.target(ui),
            // The sidebar's "+": an empty playlist.
            -2 => v.pending_add = None,
            i => {
                let playlist = usize::try_from(i)
                    .ok()
                    .and_then(|i| v.library.borrow().add_choice(i).cloned());
                if let (Some(playlist), Some(track)) = (playlist, v.target(ui)) {
                    send(Cmd::AddTo(playlist, track), tx);
                }
            }
        }
    });
}

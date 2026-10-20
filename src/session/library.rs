//! Library edits: account, sync, ratings, playlists, followed artists /
//! saved albums and downloads. Writes go to YouTube first (when they have a
//! YouTube side) and to the local library when they succeed.

use std::{collections::HashSet, path::Path, sync::Arc, time::Duration};

use anyhow::Result;

use super::{Background, Changes, Session, not_signed_in};
use crate::{
    account::{Flow, Prompt},
    api::{
        client::{Rating, YouTubeClient},
        models::{LIKED_PLAYLIST_ID, Playlist, Track},
    },
    catalog::{Catalog, Item, ItemKind},
    sync::{sync_library, sync_with_session},
    tr, trf,
};

/// Where a library write goes: the youtube.com session (InnerTube, the
/// endpoints the YouTube Music site itself uses) when there is one, else the
/// Data API client.
enum Writer {
    Session(Catalog),
    Api(Arc<YouTubeClient>),
}

impl Writer {
    async fn set_rating(&self, video_id: &str, rating: Rating) -> Result<()> {
        match self {
            Self::Session(catalog) => catalog.rate(video_id, rating).await,
            Self::Api(yt) => yt.set_rating(video_id, rating).await,
        }
    }

    /// Returns the new entry's id (`setVideoId` / playlist item id).
    async fn add_to_playlist(&self, playlist_id: &str, video_id: &str) -> Result<String> {
        match self {
            Self::Session(catalog) => catalog.add_to_playlist(playlist_id, video_id).await,
            Self::Api(yt) => yt.add_to_playlist(playlist_id, video_id).await,
        }
    }

    async fn remove_from_playlist(
        &self,
        playlist_id: &str,
        video_id: &str,
        item_id: &str,
    ) -> Result<()> {
        match self {
            Self::Session(catalog) => {
                catalog
                    .remove_from_playlist(playlist_id, video_id, item_id)
                    .await
            }
            Self::Api(yt) => yt.remove_from_playlist(item_id).await,
        }
    }

    async fn create_playlist(&self, title: &str) -> Result<String> {
        match self {
            Self::Session(catalog) => catalog.create_playlist(title).await,
            Self::Api(yt) => yt.create_playlist(title).await,
        }
    }

    /// A new private playlist holding `tracks`; returns its id.
    async fn create_playlist_with(&self, title: &str, tracks: &[Track]) -> Result<String> {
        match self {
            Self::Session(catalog) => {
                let ids: Vec<&str> = tracks.iter().map(|t| &*t.video_id).collect();
                catalog.create_playlist_with(title, &ids).await
            }
            Self::Api(yt) => {
                let id = yt.create_playlist(title).await?;
                for track in tracks {
                    yt.add_to_playlist(&id, &track.video_id).await?;
                }
                Ok(id)
            }
        }
    }

    /// Moves the entry `item_id` before `before` (to the end when `None`).
    async fn move_in_playlist(
        &self,
        playlist_id: &str,
        item_id: &str,
        before: Option<&str>,
    ) -> Result<()> {
        match self {
            Self::Session(catalog) => catalog.move_in_playlist(playlist_id, item_id, before).await,
            Self::Api(_) => anyhow::bail!("reordering on YouTube needs Sign in with Google"),
        }
    }

    async fn rename_playlist(&self, playlist_id: &str, title: &str) -> Result<()> {
        match self {
            Self::Session(catalog) => catalog.rename_playlist(playlist_id, title).await,
            Self::Api(yt) => yt.rename_playlist(playlist_id, title).await,
        }
    }

    async fn delete_playlist(&self, playlist_id: &str) -> Result<()> {
        match self {
            Self::Session(catalog) => catalog.delete_playlist(playlist_id).await,
            Self::Api(yt) => yt.delete_playlist(playlist_id).await,
        }
    }

    async fn subscribe(&self, channel_id: &str, on: bool) -> Result<()> {
        match self {
            Self::Session(catalog) => catalog.subscribe(channel_id, on).await,
            Self::Api(yt) if on => yt.subscribe(channel_id).await,
            Self::Api(yt) => yt.unsubscribe(channel_id).await,
        }
    }
}

impl Session {
    pub(super) fn library_event(&mut self, event: Background, changes: &mut Changes) {
        let lib = self.deps.library.clone();
        match event {
            Background::Synced(result) => {
                self.syncing = false;
                match result {
                    Ok(r) => {
                        changes.library = true;
                        changes.saved = true;
                        self.set_info(r.summary(self.lang));
                    }
                    Err(err) => self.set_error(trf!(
                        self.lang,
                        "Sync failed: {err:#}",
                        "Синхронізація не вдалась: {err:#}"
                    )),
                }
            }
            Background::Rated {
                track,
                rating,
                result,
            } => match result {
                Ok(()) => {
                    let stored = match rating {
                        Rating::Like => lib.add_track(LIKED_PLAYLIST_ID, &track, true),
                        Rating::None | Rating::Dislike => {
                            lib.remove_track(LIKED_PLAYLIST_ID, &track.video_id)
                        }
                    };
                    if let Err(err) = stored {
                        tracing::warn!(%err, "updating local likes");
                    }
                    changes.playlist = Some(LIKED_PLAYLIST_ID.into());
                    changes.liked = Some(track.video_id.clone());
                    self.set_info(match rating {
                        Rating::Like => {
                            trf!(self.lang, "♥ Liked {}", "♥ Вподобано {}", track.title)
                        }
                        Rating::None => trf!(
                            self.lang,
                            "Removed like: {}",
                            "Вподобайку знято: {}",
                            track.title
                        ),
                        Rating::Dislike => {
                            trf!(self.lang, "Disliked {}", "Не подобається {}", track.title)
                        }
                    });
                    // A disliked track shouldn't keep playing.
                    if rating == Rating::Dislike
                        && self
                            .queue
                            .current()
                            .is_some_and(|t| t.video_id == track.video_id)
                    {
                        self.skip(1);
                    }
                }
                Err(err) => self.set_error(format!("{err:#}")),
            },
            Background::Added {
                playlist,
                track,
                result,
            } => match result {
                Ok(item_id) => {
                    if let Err(err) = lib.add_item(&playlist.id, &track, false, Some(&item_id)) {
                        tracing::warn!(%err, "updating local playlist");
                    }
                    self.set_info(trf!(
                        self.lang,
                        "Added {} to {}",
                        "Додано {} до {}",
                        track.title,
                        playlist.title
                    ));
                    changes.playlist = Some(playlist.id);
                }
                Err(err) => self.set_error(format!("{err:#}")),
            },
            Background::Removed {
                playlist,
                track,
                result,
            } => match result {
                Ok(()) => {
                    if let Err(err) = lib.remove_track(&playlist.id, &track.video_id) {
                        tracing::warn!(%err, "updating local playlist");
                    }
                    self.set_info(trf!(
                        self.lang,
                        "Removed {} from {}",
                        "Прибрано {} з {}",
                        track.title,
                        playlist.title
                    ));
                    changes.playlist = Some(playlist.id);
                }
                Err(err) => self.set_error(format!("{err:#}")),
            },
            Background::PlaylistCreated {
                title,
                then_add,
                result,
            } => match result {
                Ok(id) => {
                    let playlist = Playlist {
                        id,
                        title,
                        item_count: 0,
                        etag: None,
                    };
                    if let Err(err) = lib.add_playlist(&playlist) {
                        tracing::warn!(%err, "storing the new playlist");
                    }
                    changes.library = true;
                    self.set_info(trf!(self.lang, "Created {}", "Створено {}", playlist.title));
                    if let Some(track) = then_add {
                        self.add_to_playlist(playlist, track);
                    }
                }
                Err(err) => self.set_error(format!("{err:#}")),
            },
            Background::PlaylistRenamed { id, title, result } => match result {
                Ok(()) => {
                    if let Err(err) = lib.rename_playlist(&id, &title) {
                        tracing::warn!(%err, "renaming locally");
                    }
                    changes.playlist = Some(id);
                    self.set_info(trf!(
                        self.lang,
                        "Renamed to {title}",
                        "Перейменовано на {title}"
                    ));
                }
                Err(err) => self.set_error(format!("{err:#}")),
            },
            Background::PlaylistDeleted { playlist, result } => match result {
                Ok(()) => {
                    if let Err(err) = lib.delete_playlist(&playlist.id) {
                        tracing::warn!(%err, "deleting locally");
                    }
                    changes.library = true;
                    self.set_info(trf!(self.lang, "Deleted {}", "Видалено {}", playlist.title));
                }
                Err(err) => self.set_error(format!("{err:#}")),
            },
            Background::Followed {
                item,
                follow,
                result,
            } => {
                // Saved locally either way; YouTube's side is best effort.
                if let Err(err) = result {
                    tracing::warn!(%err, "YouTube subscription");
                }
                let stored = if follow {
                    lib.save(&item)
                } else {
                    lib.unsave(&item.id)
                };
                match stored {
                    Ok(()) => {
                        changes.saved = true;
                        let verb = match (follow, item.kind) {
                            (true, ItemKind::Artist) => tr!(self.lang, "Following", "Стежу за"),
                            (false, ItemKind::Artist) => {
                                tr!(self.lang, "Unfollowed", "Більше не стежу за")
                            }
                            (true, _) => tr!(self.lang, "Saved", "Збережено"),
                            (false, _) => tr!(self.lang, "Removed", "Прибрано"),
                        };
                        self.set_info(trf!(self.lang, "{verb} {}", "{verb} {}", item.title));
                    }
                    Err(err) => self.set_error(format!("{err:#}")),
                }
            }
            Background::Downloaded { track, result } => {
                self.downloading.remove(&track.video_id);
                changes.downloads = true;
                match result {
                    Ok(d) => self.set_info(trf!(
                        self.lang,
                        "Downloaded {} ({:.1} MB)",
                        "Завантажено {} ({:.1} МБ)",
                        track.title,
                        d.bytes as f64 / 1e6
                    )),
                    Err(err) => self.set_error(trf!(
                        self.lang,
                        "Download failed: {err:#}",
                        "Завантаження не вдалось: {err:#}"
                    )),
                }
            }
            Background::VideoClosed { video_id, result } => {
                self.watching = false;
                match result {
                    // Carry on in audio where the video stopped.
                    Ok(at) if self.queue.current().is_some_and(|t| t.video_id == video_id) => {
                        self.seek_to(self.song_time(at));
                        self.player.set_paused(false);
                        self.set_info(tr!(self.lang, "Back to audio", "Назад до аудіо"));
                    }
                    Ok(_) => {}
                    Err(err) => self.set_error(trf!(self.lang, "Video: {err:#}", "Відео: {err:#}")),
                }
            }
            Background::QueueSaved {
                title,
                tracks,
                result,
            } => match result {
                Ok(id) => {
                    let playlist = Playlist {
                        id: id.clone(),
                        title: title.clone(),
                        item_count: 0,
                        etag: None,
                    };
                    let stored = lib.add_playlist(&playlist).and_then(|()| {
                        tracks
                            .iter()
                            .try_for_each(|t| lib.add_item(&id, t, false, None))
                    });
                    if let Err(err) = stored {
                        tracing::warn!(%err, "storing the saved queue");
                    }
                    changes.library = true;
                    self.set_info(trf!(
                        self.lang,
                        "Saved {} songs as {title}",
                        "Збережено пісень: {} — плейлист «{title}»",
                        tracks.len()
                    ));
                }
                Err(err) => self.set_error(format!("{err:#}")),
            },
            Background::Moved {
                playlist_id,
                from,
                to,
                result,
            } => {
                if let Err(err) = result {
                    // YouTube kept the old order: so does the library.
                    if let Err(undo) = lib.move_track(&playlist_id, to, from) {
                        tracing::warn!(%undo, "moving the song back");
                    }
                    self.set_error(format!("{err:#}"));
                }
                changes.playlist = Some(playlist_id);
            }
            Background::HistoryEdited(result) => {
                changes.history = true;
                if let Err(err) = result {
                    self.set_error(format!("{err:#}"));
                }
            }
            Background::DownloadsStopped { stopped } => {
                changes.downloads = true;
                self.set_info(match stopped {
                    1 => tr!(self.lang, "Stopped the download", "Завантаження зупинено").to_owned(),
                    n => trf!(
                        self.lang,
                        "Stopped {n} downloads",
                        "Зупинено завантажень: {n}"
                    ),
                });
            }
            Background::DownloadsCleared(result) => {
                changes.downloads = true;
                match result {
                    Ok(n) => self.set_info(trf!(
                        self.lang,
                        "Deleted {n} downloaded song(s)",
                        "Видалено завантажених пісень: {n}"
                    )),
                    Err(err) => self.set_error(format!("{err:#}")),
                }
            }
            Background::DownloadRemoved { video_id, result } => {
                changes.downloads = true;
                if let Err(err) = result {
                    self.set_error(format!("{err:#}"));
                } else {
                    tracing::debug!(%video_id, "download removed");
                }
            }
            Background::SignedIn { generation, result } if generation == self.login_generation => {
                self.signing_in = false;
                changes.account = true;
                match result {
                    Ok(()) => {
                        self.deps.account.connect();
                        self.set_info(tr!(self.lang, "Signed in", "Вхід виконано"));
                        self.start_sync();
                    }
                    Err(err) => self.set_error(trf!(
                        self.lang,
                        "Sign-in failed: {err:#}",
                        "Вхід не вдався: {err:#}"
                    )),
                }
            }
            Background::SignedIn { .. } => {} // a newer attempt replaced it
            Background::SignedOut(result) => {
                changes.account = true;
                match result {
                    Ok(()) => self.set_info(tr!(
                        self.lang,
                        "Signed out — the library stays available offline",
                        "Вихід виконано — бібліотека лишається офлайн"
                    )),
                    Err(err) => self.set_error(trf!(
                        self.lang,
                        "Sign-out failed: {err:#}",
                        "Вихід не вдався: {err:#}"
                    )),
                }
            }
            _ => unreachable!("not a library event"),
        }
    }

    fn youtube(&mut self) -> Option<Arc<YouTubeClient>> {
        if !self.signed_in() {
            self.set_error(not_signed_in(self.lang));
        }
        self.deps.account.youtube()
    }

    /// The sign-in library writes go through: the session when there is one,
    /// else the Data API client. Quiet: callers report being signed out.
    fn writer(&self) -> Option<Writer> {
        if self.deps.youtube.signed_in() {
            return Some(Writer::Session(self.deps.catalog.clone()));
        }
        self.deps.account.youtube().map(Writer::Api)
    }

    /// Runs `work` against the account in the background and reports its
    /// `Background` event; shows `busy` meanwhile. Nothing happens when
    /// signed out.
    fn with_youtube<F, Fut>(&mut self, busy: impl Into<String>, work: F)
    where
        F: FnOnce(Writer) -> Fut,
        Fut: std::future::Future<Output = Background> + Send + 'static,
    {
        let Some(writer) = self.writer() else {
            self.set_error(not_signed_in(self.lang));
            return;
        };
        self.set_info(busy);
        let tx = self.tx.clone();
        let task = work(writer);
        tokio::spawn(async move {
            let _ = tx.send(task.await);
        });
    }

    pub fn start_sync(&mut self) {
        if self.syncing {
            return;
        }
        let (library, tx) = (self.deps.library.clone(), self.tx.clone());
        // The youtube.com sign-in reads the library for free and never
        // expires; the Data API is only for accounts that have no session.
        if self.deps.youtube.signed_in() {
            let catalog = self.deps.catalog.clone();
            self.syncing = true;
            self.set_info(tr!(
                self.lang,
                "Syncing library…",
                "Синхронізую бібліотеку…"
            ));
            tokio::spawn(async move {
                let result = sync_with_session(&catalog, library).await;
                let _ = tx.send(Background::Synced(result));
            });
            return;
        }
        let Some(youtube) = self.youtube() else {
            return;
        };
        self.syncing = true;
        self.set_info(tr!(
            self.lang,
            "Syncing library…",
            "Синхронізую бібліотеку…"
        ));
        let music_only = self.deps.liked_music_only;
        tokio::spawn(async move {
            let result = sync_library(&youtube, library, music_only).await;
            let _ = tx.send(Background::Synced(result));
        });
    }

    // --- ratings --------------------------------------------------------------------

    pub fn is_liked(&self, video_id: &str) -> bool {
        self.deps
            .library
            .contains(LIKED_PLAYLIST_ID, video_id)
            .unwrap_or(false)
    }

    /// Likes `track`, or removes the like if it's already liked.
    pub fn toggle_like(&mut self, track: Track) {
        let rating = if self.is_liked(&track.video_id) {
            Rating::None
        } else {
            Rating::Like
        };
        self.rate(track, rating);
    }

    /// Dislikes `track` (YouTube learns from it; it leaves Liked music and
    /// is skipped when playing).
    pub fn dislike(&mut self, track: Track) {
        self.rate(track, Rating::Dislike);
    }

    fn rate(&mut self, track: Track, rating: Rating) {
        let busy = match rating {
            Rating::Like => tr!(self.lang, "Liking…", "Ставлю вподобайку…"),
            Rating::None => tr!(self.lang, "Removing like…", "Знімаю вподобайку…"),
            Rating::Dislike => tr!(self.lang, "Disliking…", "Ставлю «не подобається»…"),
        };
        self.with_youtube(busy, move |yt| async move {
            let result = yt.set_rating(&track.video_id, rating).await;
            Background::Rated {
                track,
                rating,
                result,
            }
        });
    }

    /// Whether like / add-to-playlist are possible (shows why not).
    pub fn can_edit(&mut self) -> bool {
        if self.writer().is_none() {
            self.set_error(not_signed_in(self.lang));
            return false;
        }
        true
    }

    // --- playlists ------------------------------------------------------------------

    pub fn add_to_playlist(&mut self, playlist: Playlist, track: Track) {
        let busy = trf!(self.lang, "Adding to {}…", "Додаю до {}…", playlist.title);
        self.with_youtube(busy, move |yt| async move {
            let result = yt.add_to_playlist(&playlist.id, &track.video_id).await;
            Background::Added {
                playlist,
                track,
                result,
            }
        });
    }

    /// Removes `track` from one of the user's playlists.
    pub fn remove_from_playlist(&mut self, playlist: Playlist, track: Track) {
        if playlist.id == LIKED_PLAYLIST_ID {
            return self.rate(track, Rating::None);
        }
        let item_id = self
            .deps
            .library
            .item_id(&playlist.id, &track.video_id)
            .ok()
            .flatten();
        let Some(item_id) = item_id else {
            self.set_error(tr!(
                self.lang,
                "Sync the library first: this entry's id isn't known yet",
                "Спершу синхронізуй бібліотеку: id цього запису ще невідомий"
            ));
            return;
        };
        let busy = trf!(
            self.lang,
            "Removing from {}…",
            "Прибираю з {}…",
            playlist.title
        );
        self.with_youtube(busy, move |yt| async move {
            let result = yt
                .remove_from_playlist(&playlist.id, &track.video_id, &item_id)
                .await;
            Background::Removed {
                playlist,
                track,
                result,
            }
        });
    }

    /// Creates a private playlist, optionally adding `track` to it.
    pub fn create_playlist(&mut self, title: &str, then_add: Option<Track>) {
        let title = title.trim().to_owned();
        if title.is_empty() {
            return;
        }
        let busy = trf!(self.lang, "Creating {title}…", "Створюю {title}…");
        self.with_youtube(busy, move |yt| async move {
            let result = yt.create_playlist(&title).await;
            Background::PlaylistCreated {
                title,
                then_add,
                result,
            }
        });
    }

    pub fn rename_playlist(&mut self, playlist: Playlist, title: &str) {
        let title = title.trim().to_owned();
        if title.is_empty() || playlist.id == LIKED_PLAYLIST_ID {
            return;
        }
        let busy = tr!(self.lang, "Renaming…", "Перейменовую…");
        self.with_youtube(busy, move |yt| async move {
            let result = yt.rename_playlist(&playlist.id, &title).await;
            Background::PlaylistRenamed {
                id: playlist.id,
                title,
                result,
            }
        });
    }

    pub fn delete_playlist(&mut self, playlist: Playlist) {
        if playlist.id == LIKED_PLAYLIST_ID {
            return;
        }
        let busy = trf!(self.lang, "Deleting {}…", "Видаляю {}…", playlist.title);
        self.with_youtube(busy, move |yt| async move {
            let result = yt.delete_playlist(&playlist.id).await;
            Background::PlaylistDeleted { playlist, result }
        });
    }

    /// Saves what is playing and up next as a new private playlist.
    pub fn save_queue_as_playlist(&mut self, title: &str) {
        let title = title.trim().to_owned();
        if title.is_empty() {
            return;
        }
        let tracks: Vec<Track> = self
            .queue
            .current()
            .into_iter()
            .chain(self.queue.upcoming())
            .cloned()
            .collect();
        if tracks.is_empty() {
            self.set_error(tr!(self.lang, "The queue is empty", "Черга порожня"));
            return;
        }
        let busy = trf!(
            self.lang,
            "Saving the queue as {title}…",
            "Зберігаю чергу як «{title}»…"
        );
        self.with_youtube(busy, move |yt| async move {
            let result = yt.create_playlist_with(&title, &tracks).await;
            Background::QueueSaved {
                title,
                tracks,
                result,
            }
        });
    }

    /// Moves the song at `from` to `to` in one of the user's playlists: here
    /// at once, then on YouTube (back again if YouTube refuses).
    pub fn move_in_playlist(&mut self, playlist: Playlist, from: usize, to: usize) {
        if playlist.id == LIKED_PLAYLIST_ID || from == to {
            return;
        }
        let lib = self.deps.library.clone();
        let mut ids = lib.item_ids(&playlist.id).unwrap_or_default();
        if from >= ids.len() || to >= ids.len() {
            return;
        }
        if let Err(err) = lib.move_track(&playlist.id, from, to) {
            return self.set_error(format!("{err:#}"));
        }
        let moved = ids.remove(from);
        ids.insert(to, moved.clone());
        let before = ids.get(to + 1).cloned().filter(|id| !id.is_empty());
        let tx = self.tx.clone();
        let playlist_id = playlist.id.clone();
        if moved.is_empty() {
            // Not synced yet: YouTube's id for this entry is unknown.
            self.set_info(tr!(
                self.lang,
                "Moved here; sync the library to move it on YouTube too",
                "Переміщено тут; синхронізуй бібліотеку, щоб і на YouTube"
            ));
            let _ = tx.send(Background::Moved {
                playlist_id,
                from,
                to,
                result: Ok(()),
            });
            return;
        }
        let Some(writer) = self.writer() else {
            let _ = tx.send(Background::Moved {
                playlist_id,
                from,
                to,
                result: Ok(()),
            });
            return;
        };
        tokio::spawn(async move {
            let result = writer
                .move_in_playlist(&playlist_id, &moved, before.as_deref())
                .await;
            let _ = tx.send(Background::Moved {
                playlist_id,
                from,
                to,
                result,
            });
        });
    }

    /// Takes a song out of the history (and forgets how often it played).
    pub fn remove_from_history(&mut self, video_id: Arc<str>) {
        let result = self.deps.library.remove_history(&video_id);
        let _ = self.tx.send(Background::HistoryEdited(result));
    }

    /// Forgets every play.
    pub fn clear_history(&mut self) {
        let result = self.deps.library.clear_history();
        if result.is_ok() {
            self.set_info(tr!(self.lang, "History cleared", "Історію очищено"));
        }
        let _ = self.tx.send(Background::HistoryEdited(result));
    }

    // --- followed artists, saved albums -------------------------------------------

    pub fn is_saved(&self, id: &str) -> bool {
        self.deps.library.is_saved(id).unwrap_or(false)
    }

    /// Follows / unfollows an artist (also subscribing on YouTube when
    /// signed in and the channel is known) or saves / removes an album or
    /// playlist in the library.
    pub fn toggle_saved(&mut self, item: Item, channel_id: Option<String>) {
        let follow = !self.is_saved(&item.id);
        let writer = (item.kind == ItemKind::Artist)
            .then(|| self.writer())
            .flatten();
        let channel = channel_id.unwrap_or_else(|| item.id.to_string());
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = match writer {
                Some(writer) => writer.subscribe(&channel, follow).await,
                None => Ok(()),
            };
            let _ = tx.send(Background::Followed {
                item,
                follow,
                result,
            });
        });
    }

    // --- downloads ------------------------------------------------------------------

    /// Video ids of the downloaded songs.
    pub(super) fn downloaded_ids(&self) -> HashSet<String> {
        self.deps
            .library
            .downloads()
            .map(|d| {
                d.into_iter()
                    .map(|d| d.track.video_id.to_string())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Lyrics for downloaded songs that don't have them on disk yet (songs
    /// downloaded before lyrics were kept, or offline back then). Quiet:
    /// whatever fails is tried again at the next start.
    pub(super) fn fetch_download_lyrics(&self) {
        let missing: Vec<Track> = self
            .deps
            .library
            .downloads()
            .unwrap_or_default()
            .into_iter()
            .map(|d| d.track)
            .filter(|t| !self.deps.lyrics_cache.contains(&t.video_id))
            .collect();
        if missing.is_empty() {
            return;
        }
        let (http, catalog, cache) = (
            self.deps.http.clone(),
            self.deps.catalog.clone(),
            self.deps.lyrics_cache.clone(),
        );
        let downloaded = self.downloaded_ids();
        tokio::spawn(async move {
            for track in missing {
                keep_lyrics(&http, &catalog, &cache, &track, &downloaded).await;
            }
        });
    }

    pub fn is_downloaded(&self, video_id: &str) -> bool {
        self.deps
            .library
            .download_path(video_id)
            .ok()
            .flatten()
            .is_some()
    }

    /// Saves tracks for offline play (one yt-dlp at a time, in order).
    pub fn download(&mut self, tracks: Vec<Track>) {
        let limit = self.deps.download_limit;
        let used: u64 = self
            .deps
            .library
            .downloads()
            .map(|d| d.iter().map(|d| d.bytes).sum())
            .unwrap_or(0);
        if limit.is_some_and(|l| used >= l) {
            self.set_error(tr!(
                self.lang,
                "Download limit reached (download_limit_mb in config)",
                "Ліміт завантажень вичерпано (download_limit_mb у конфігу)"
            ));
            return;
        }
        let todo: Vec<Track> = tracks
            .into_iter()
            .filter(|t| {
                !self.is_downloaded(&t.video_id) && self.downloading.insert(t.video_id.clone())
            })
            .collect();
        if todo.is_empty() {
            return;
        }
        self.set_info(trf!(
            self.lang,
            "Downloading {} track(s)…",
            "Завантажую треків: {}…",
            todo.len()
        ));
        let (source, tx) = (self.deps.source.clone(), self.tx.clone());
        let (http, catalog, cache) = (
            self.deps.http.clone(),
            self.deps.catalog.clone(),
            self.deps.lyrics_cache.clone(),
        );
        let mut downloaded = self.downloaded_ids();
        // One task for the batch, so stopping it stops the rest as well.
        self.download_task = Some(tokio::spawn(async move {
            for track in todo {
                let result = source.download(&track).await;
                if result.is_ok() {
                    // Its lyrics go with it, for offline play.
                    downloaded.insert(track.video_id.to_string());
                    keep_lyrics(&http, &catalog, &cache, &track, &downloaded).await;
                }
                let _ = tx.send(Background::Downloaded { track, result });
            }
        }));
    }

    pub fn remove_download(&mut self, video_id: Arc<str>) {
        self.deps.lyrics_cache.remove(&video_id);
        let (source, tx) = (self.deps.source.clone(), self.tx.clone());
        tokio::spawn(async move {
            let result = source.remove_download(&video_id).await;
            let _ = tx.send(Background::DownloadRemoved { video_id, result });
        });
    }

    /// Stops the downloads that are queued or running (yt-dlp is killed with
    /// its task) and throws away the half-written file.
    pub fn cancel_downloads(&mut self) {
        if self.downloading.is_empty() {
            return;
        }
        if let Some(task) = self.download_task.take() {
            task.abort();
        }
        let stopped = self.downloading.len();
        self.downloading.clear();
        let (source, tx) = (self.deps.source.clone(), self.tx.clone());
        tokio::spawn(async move {
            if let Err(err) = source.clean_downloads().await {
                tracing::warn!(%err, "cleaning up after cancelled downloads");
            }
            let _ = tx.send(Background::DownloadsStopped { stopped });
        });
    }

    /// Deletes every downloaded song (stopping downloads that still run).
    pub fn remove_all_downloads(&mut self) {
        self.cancel_downloads();
        for id in self.downloaded_ids() {
            self.deps.lyrics_cache.remove(&id);
        }
        let (source, tx) = (self.deps.source.clone(), self.tx.clone());
        tokio::spawn(async move {
            let result = source.remove_all_downloads().await;
            let _ = tx.send(Background::DownloadsCleared(result));
        });
    }

    // --- account ------------------------------------------------------------------

    /// Signed in to YouTube ("Sign in with Google"): what the player needs
    /// before it shows anything (it shows only the account's own music).
    pub fn youtube_signed_in(&self) -> bool {
        self.deps.youtube.signed_in()
    }

    /// Forgets the (made-up) YouTube sign-in of the test session.
    #[cfg(test)]
    pub(crate) fn sign_out_for_tests(&mut self) {
        self.deps.youtube = crate::youtube_session::YouTubeSession::none();
    }

    pub fn signed_in(&self) -> bool {
        self.deps.account.signed_in()
    }

    /// The configured OAuth client ID (browser sign-in), if any.
    pub fn client_id(&self) -> Option<&str> {
        self.deps.account.client_id(Flow::Browser)
    }

    /// Saves an OAuth client ("Desktop app") into `config.toml`.
    pub fn set_client(&mut self, id: &str, secret: &str) {
        let result = self.deps.account.set_client(Flow::Browser, id, secret);
        self.client_saved(result);
    }

    /// Reads Google's downloaded client JSON and saves it.
    pub fn import_client(&mut self, path: &Path) {
        let result = self.deps.account.import_client(Flow::Browser, path);
        self.client_saved(result.map(|_| ()));
    }

    fn client_saved(&mut self, result: Result<()>) {
        match result {
            Ok(()) if self.signed_in() => self.set_info(tr!(
                self.lang,
                "OAuth client saved",
                "OAuth-клієнт збережено"
            )),
            Ok(()) => self.set_info(tr!(
                self.lang,
                "OAuth client saved — now sign in",
                "OAuth-клієнт збережено — тепер увійди"
            )),
            Err(err) => self.set_error(format!("{err:#}")),
        }
    }

    /// Opens Google's consent page in the browser and waits (in the
    /// background) for the redirect; then syncs.
    pub fn sign_in(&mut self) {
        let signing_in = self.deps.account.sign_in(Flow::Browser, |prompt| {
            if let Prompt::OpenUrl(url) = prompt {
                tracing::info!(%url, "sign-in page");
            }
        });
        let signing_in = match signing_in {
            Ok(future) => future,
            Err(err) => {
                self.set_error(format!("{err:#}"));
                return;
            }
        };
        self.login_generation += 1;
        self.signing_in = true;
        self.set_info(tr!(
            self.lang,
            "Finish signing in in your browser…",
            "Заверши вхід у браузері…"
        ));
        let (tx, generation) = (self.tx.clone(), self.login_generation);
        tokio::spawn(async move {
            let result = signing_in.await;
            let _ = tx.send(Background::SignedIn { generation, result });
        });
    }

    /// Forgets the stored token; the library stays for offline use.
    pub fn sign_out(&mut self) {
        self.login_generation += 1;
        self.signing_in = false;
        let clearing = self.deps.account.sign_out();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let _ = tx.send(Background::SignedOut(clearing.await));
        });
    }
}

/// Finds a song's lyrics and keeps them on disk, unless they already are.
async fn keep_lyrics(
    http: &reqwest::Client,
    catalog: &crate::catalog::Catalog,
    cache: &crate::lyrics::Cache,
    track: &Track,
    keep: &HashSet<String>,
) {
    if cache.contains(&track.video_id) {
        return;
    }
    let duration = track.duration_secs.map(|s| Duration::from_secs(s.into()));
    match crate::lyrics::find(http, catalog, track, duration).await {
        Ok(Some(found)) => cache.put(&track.video_id, &found, |id| keep.contains(id)),
        Ok(None) => {}
        Err(err) => tracing::debug!(%err, video_id = %track.video_id, "lyrics for a download"),
    }
}

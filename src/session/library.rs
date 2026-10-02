//! Library edits: account, sync, ratings, playlists, followed artists /
//! saved albums and downloads. Writes go to YouTube first (when they have a
//! YouTube side) and to the local library when they succeed.

use std::{path::Path, sync::Arc};

use anyhow::Result;

use super::{Background, Changes, NOT_SIGNED_IN, Session};
use crate::{
    account::{Flow, Prompt},
    api::{
        client::{Rating, YouTubeClient},
        models::{LIKED_PLAYLIST_ID, Playlist, Track},
    },
    catalog::{Item, ItemKind},
    sync::sync_library,
};

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
                        self.set_info(r.summary());
                    }
                    Err(err) => self.set_error(format!("Sync failed: {err:#}")),
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
                        Rating::Like => format!("♥ Liked {}", track.title),
                        Rating::None => format!("Removed like: {}", track.title),
                        Rating::Dislike => format!("Disliked {}", track.title),
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
                    self.set_info(format!("Added {} to {}", track.title, playlist.title));
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
                    self.set_info(format!("Removed {} from {}", track.title, playlist.title));
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
                    self.set_info(format!("Created {}", playlist.title));
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
                    self.set_info(format!("Renamed to {title}"));
                }
                Err(err) => self.set_error(format!("{err:#}")),
            },
            Background::PlaylistDeleted { playlist, result } => match result {
                Ok(()) => {
                    if let Err(err) = lib.delete_playlist(&playlist.id) {
                        tracing::warn!(%err, "deleting locally");
                    }
                    changes.library = true;
                    self.set_info(format!("Deleted {}", playlist.title));
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
                            (true, ItemKind::Artist) => "Following",
                            (false, ItemKind::Artist) => "Unfollowed",
                            (true, _) => "Saved",
                            (false, _) => "Removed",
                        };
                        self.set_info(format!("{verb} {}", item.title));
                    }
                    Err(err) => self.set_error(format!("{err:#}")),
                }
            }
            Background::Downloaded { track, result } => {
                self.downloading.remove(&track.video_id);
                changes.downloads = true;
                match result {
                    Ok(d) => self.set_info(format!(
                        "Downloaded {} ({:.1} MB)",
                        track.title,
                        d.bytes as f64 / 1e6
                    )),
                    Err(err) => self.set_error(format!("Download failed: {err:#}")),
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
                        self.set_info("Signed in");
                        self.start_sync();
                    }
                    Err(err) => self.set_error(format!("Sign-in failed: {err:#}")),
                }
            }
            Background::SignedIn { .. } => {} // a newer attempt replaced it
            Background::SignedOut(result) => {
                changes.account = true;
                match result {
                    Ok(()) => self.set_info("Signed out — the library stays available offline"),
                    Err(err) => self.set_error(format!("Sign-out failed: {err:#}")),
                }
            }
            _ => unreachable!("not a library event"),
        }
    }

    fn youtube(&mut self) -> Option<Arc<YouTubeClient>> {
        if !self.signed_in() {
            self.set_error(NOT_SIGNED_IN);
        }
        self.deps.account.youtube()
    }

    /// Runs `work` against the API in the background and reports through
    /// `done`; shows `busy` meanwhile. Nothing happens when signed out.
    fn with_youtube<F, Fut>(&mut self, busy: impl Into<String>, work: F)
    where
        F: FnOnce(Arc<YouTubeClient>) -> Fut,
        Fut: std::future::Future<Output = Background> + Send + 'static,
    {
        let Some(youtube) = self.youtube() else {
            return;
        };
        self.set_info(busy);
        let tx = self.tx.clone();
        let task = work(youtube);
        tokio::spawn(async move {
            let _ = tx.send(task.await);
        });
    }

    pub fn start_sync(&mut self) {
        if self.syncing {
            return;
        }
        let Some(youtube) = self.youtube() else {
            return;
        };
        self.syncing = true;
        self.set_info("Syncing library…");
        let (library, music_only, tx) = (
            self.deps.library.clone(),
            self.deps.liked_music_only,
            self.tx.clone(),
        );
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
            Rating::Like => "Liking…",
            Rating::None => "Removing like…",
            Rating::Dislike => "Disliking…",
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
        self.youtube().is_some()
    }

    // --- playlists ------------------------------------------------------------------

    pub fn add_to_playlist(&mut self, playlist: Playlist, track: Track) {
        let busy = format!("Adding to {}…", playlist.title);
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
            self.set_error("Sync the library first: this entry's id isn't known yet");
            return;
        };
        let busy = format!("Removing from {}…", playlist.title);
        self.with_youtube(busy, move |yt| async move {
            let result = yt.remove_from_playlist(&item_id).await;
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
        let busy = format!("Creating {title}…");
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
        self.with_youtube("Renaming…", move |yt| async move {
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
        let busy = format!("Deleting {}…", playlist.title);
        self.with_youtube(busy, move |yt| async move {
            let result = yt.delete_playlist(&playlist.id).await;
            Background::PlaylistDeleted { playlist, result }
        });
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
        let youtube = (item.kind == ItemKind::Artist)
            .then(|| self.deps.account.youtube())
            .flatten();
        let channel = channel_id.unwrap_or_else(|| item.id.to_string());
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = match youtube {
                Some(yt) if follow => yt.subscribe(&channel).await,
                Some(yt) => yt.unsubscribe(&channel).await,
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
            self.set_error("Download limit reached (download_limit_mb in config)");
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
        self.set_info(format!("Downloading {} track(s)…", todo.len()));
        let (source, tx) = (self.deps.source.clone(), self.tx.clone());
        tokio::spawn(async move {
            for track in todo {
                let result = source.download(&track).await;
                let _ = tx.send(Background::Downloaded { track, result });
            }
        });
    }

    pub fn remove_download(&mut self, video_id: Arc<str>) {
        let (source, tx) = (self.deps.source.clone(), self.tx.clone());
        tokio::spawn(async move {
            let result = source.remove_download(&video_id).await;
            let _ = tx.send(Background::DownloadRemoved { video_id, result });
        });
    }

    // --- account ------------------------------------------------------------------

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
            Ok(()) if self.signed_in() => self.set_info("OAuth client saved"),
            Ok(()) => self.set_info("OAuth client saved — now sign in"),
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
        self.set_info("Finish signing in in your browser…");
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

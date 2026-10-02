//! Account sync: YouTube Data API -> local library. Runs on first start (empty
//! library) and on demand, never on a timer, to keep quota use minimal.
//! Playlists whose ETag hasn't changed reuse their stored tracks instead of
//! re-fetching items (1 unit per 50 tracks saved each).

use std::sync::Arc;

use anyhow::Result;

use crate::{
    api::{
        client::{LikedSource, YouTubeClient},
        models::{LIKED_PLAYLIST_ID, Playlist},
    },
    storage::{Library, SyncedPlaylist},
};

#[derive(Debug, Clone)]
pub struct SyncReport {
    pub playlists: usize,
    pub tracks: usize,
    /// Playlists skipped because their ETag was unchanged.
    pub unchanged: usize,
    pub quota_units: u32,
    pub liked: usize,
    pub liked_source: LikedSource,
}

impl SyncReport {
    /// "Synced 5 playlists, 812 tracks (3 unchanged, 21 API units) · …".
    pub fn summary(&self) -> String {
        format!(
            "Synced {} playlists, {} tracks ({} unchanged, {} API units) · {}",
            self.playlists,
            self.tracks,
            self.unchanged,
            self.quota_units,
            self.liked_note()
        )
    }

    /// One-line explanation of where the liked list came from.
    pub fn liked_note(&self) -> String {
        match self.liked_source {
            LikedSource::YouTubeMusic => format!("{} liked (YouTube Music)", self.liked),
            LikedSource::LikedVideos {
                total,
                music_only: true,
            } => format!(
                "{} of {total} liked videos are Music-category (liked_music_only = false keeps all)",
                self.liked
            ),
            LikedSource::LikedVideos { .. } => format!("{} liked videos", self.liked),
        }
    }
}

pub async fn sync_library(
    client: &YouTubeClient,
    library: Arc<Library>,
    liked_music_only: bool,
) -> Result<SyncReport> {
    let start_units = client.units_used();
    let known_etags = {
        let library = library.clone();
        tokio::task::spawn_blocking(move || library.playlist_etags()).await??
    };

    let liked = client.liked_tracks(liked_music_only).await?;
    let (liked_count, liked_source) = (liked.tracks.len(), liked.source);
    let mut all = vec![SyncedPlaylist {
        playlist: Playlist {
            id: LIKED_PLAYLIST_ID.into(),
            title: "Liked music".into(),
            item_count: liked_count as u32,
            etag: None,
        },
        tracks: liked.tracks.into(),
        item_ids: Vec::new(),
    }];
    let mut unchanged = 0;
    // "LM" may also be listed among the user's playlists; it's shown as Liked.
    for playlist in client
        .my_playlists()
        .await?
        .into_iter()
        .filter(|p| p.id != "LM" && p.id != "LL")
    {
        let same =
            playlist.etag.is_some() && playlist.etag.as_ref() == known_etags.get(&playlist.id);
        let (tracks, item_ids) = if same {
            unchanged += 1;
            (
                library.tracks(&playlist.id)?,
                library.item_ids(&playlist.id)?,
            )
        } else {
            let (tracks, ids) = client.playlist_entries(&playlist.id).await?;
            (tracks.into(), ids)
        };
        all.push(SyncedPlaylist {
            playlist,
            tracks,
            item_ids,
        });
    }
    // Followed artists = channel subscriptions; a failure keeps the old list.
    match client.subscriptions().await {
        Ok(artists) => library.replace_subscriptions(&artists)?,
        Err(err) => tracing::warn!(%err, "syncing subscriptions"),
    }

    let report = SyncReport {
        playlists: all.len(),
        tracks: all.iter().map(|p| p.tracks.len()).sum(),
        unchanged,
        quota_units: client.units_used() - start_units,
        liked: liked_count,
        liked_source,
    };
    tokio::task::spawn_blocking(move || library.replace_synced(&all)).await??;
    Ok(report)
}

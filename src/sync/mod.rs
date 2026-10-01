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
    storage::Library,
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
    let mut all = vec![(
        Playlist {
            id: LIKED_PLAYLIST_ID.into(),
            title: "Liked music".into(),
            item_count: liked_count as u32,
            etag: None,
        },
        liked.tracks.into(),
    )];
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
        let tracks = if same {
            unchanged += 1;
            library.tracks(&playlist.id)?
        } else {
            client.playlist_tracks(&playlist.id).await?.into()
        };
        all.push((playlist, tracks));
    }

    let report = SyncReport {
        playlists: all.len(),
        tracks: all.iter().map(|(_, t)| t.len()).sum(),
        unchanged,
        quota_units: client.units_used() - start_units,
        liked: liked_count,
        liked_source,
    };
    tokio::task::spawn_blocking(move || library.replace_library(&all)).await??;
    Ok(report)
}

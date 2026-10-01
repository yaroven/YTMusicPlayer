//! Account sync: YouTube Data API -> local library. Runs on first start (empty
//! library) and on demand, never on a timer, to keep quota use minimal.

use std::sync::Arc;

use anyhow::Result;

use crate::{
    api::{
        client::YouTubeClient,
        models::{LIKED_PLAYLIST_ID, Playlist},
    },
    storage::Library,
};

#[derive(Debug, Clone)]
pub struct SyncReport {
    pub playlists: usize,
    pub tracks: usize,
    pub quota_units: u32,
}

pub async fn sync_library(
    client: &YouTubeClient,
    library: Arc<Library>,
    liked_music_only: bool,
) -> Result<SyncReport> {
    let start_units = client.units_used();

    let liked = client.liked_tracks(liked_music_only).await?;
    let mut all = vec![(
        Playlist {
            id: LIKED_PLAYLIST_ID.into(),
            title: "Liked music".into(),
            item_count: liked.len() as u32,
        },
        liked,
    )];
    for playlist in client.my_playlists().await? {
        let tracks = client.playlist_tracks(&playlist.id).await?;
        all.push((playlist, tracks));
    }

    let report = SyncReport {
        playlists: all.len(),
        tracks: all.iter().map(|(_, t)| t.len()).sum(),
        quota_units: client.units_used() - start_units,
    };
    tokio::task::spawn_blocking(move || library.replace_library(&all)).await??;
    Ok(report)
}

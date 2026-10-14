//! Account sync: YouTube -> local library. Runs on first start (empty
//! library) and on demand, never on a timer.
//!
//! Two paths. [`sync_with_session`] uses the youtube.com sign-in ("Sign in
//! with Google") and YouTube Music's own InnerTube endpoints: no quota, no
//! OAuth client, and no weekly expiry. [`sync_library`] is the older Data
//! API path, kept for accounts that only set up the OAuth client; it skips
//! playlists whose ETag hasn't changed (1 unit per 50 tracks saved each).

use std::sync::Arc;

use anyhow::Result;

use crate::{
    api::{
        client::{LikedSource, YouTubeClient},
        models::{LIKED_PLAYLIST_ID, Playlist},
    },
    catalog::{Catalog, ItemKind},
    i18n::Lang,
    storage::{Library, SyncedPlaylist},
    tr, trf,
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
    /// Synced through the youtube.com session instead of the Data API.
    pub via_session: bool,
}

impl SyncReport {
    /// "Synced 5 playlists, 812 tracks (3 unchanged, 21 API units) · …".
    pub fn summary(&self, lang: Lang) -> String {
        let cost = if self.via_session {
            tr!(lang, "no API quota", "без квоти API").to_owned()
        } else {
            trf!(
                lang,
                "{} unchanged, {} API units",
                "{} без змін, {} одиниць API",
                self.unchanged,
                self.quota_units
            )
        };
        trf!(
            lang,
            "Synced {} playlists, {} tracks ({cost}) · {}",
            "Синхронізовано {} плейлистів, {} треків ({cost}) · {}",
            self.playlists,
            self.tracks,
            self.liked_note(lang)
        )
    }

    /// One-line explanation of where the liked list came from.
    pub fn liked_note(&self, lang: Lang) -> String {
        match self.liked_source {
            LikedSource::YouTubeMusic => trf!(
                lang,
                "{} liked (YouTube Music)",
                "{} вподобаних (YouTube Music)",
                self.liked
            ),
            LikedSource::LikedVideos {
                total,
                music_only: true,
            } => trf!(
                lang,
                "{} of {total} liked videos are Music-category (liked_music_only = false keeps all)",
                "{} із {total} вподобаних відео — музика (liked_music_only = false лишає всі)",
                self.liked
            ),
            LikedSource::LikedVideos { .. } => {
                trf!(lang, "{} liked videos", "{} вподобаних відео", self.liked)
            }
        }
    }
}

/// Syncs the library through the signed-in YouTube Music session: liked
/// music, playlists, saved albums and followed artists, exactly as the web
/// app sees them. Costs no API quota.
pub async fn sync_with_session(catalog: &Catalog, library: Arc<Library>) -> Result<SyncReport> {
    let liked = catalog.library_liked().await?;
    let liked_count = liked.len();
    let mut all = vec![SyncedPlaylist {
        playlist: Playlist {
            id: LIKED_PLAYLIST_ID.into(),
            title: "Liked music".into(),
            item_count: liked_count as u32,
            etag: None,
        },
        tracks: liked.into(),
        item_ids: Vec::new(),
    }];
    for item in catalog.library_playlists().await? {
        let (tracks, item_ids) = match catalog.playlist_entries(&item.id).await {
            Ok(entries) => entries,
            // One unreadable playlist shouldn't fail the whole sync.
            Err(err) => {
                tracing::warn!(%err, playlist = %item.id, "syncing playlist");
                continue;
            }
        };
        all.push(SyncedPlaylist {
            playlist: Playlist {
                // Stored without the browse prefix, like the Data API's ids.
                id: item.id.trim_start_matches("VL").into(),
                title: item.title.to_string(),
                item_count: tracks.len() as u32,
                etag: None,
            },
            tracks: tracks.into(),
            item_ids,
        });
    }
    // Saved albums and followed artists; a failure keeps the old list.
    for (kind, items) in [
        (ItemKind::Album, catalog.library_albums().await),
        (ItemKind::Artist, catalog.library_artists().await),
    ] {
        match items {
            Ok(items) => library.replace_saved_synced(kind, &items)?,
            Err(err) => tracing::warn!(%err, ?kind, "syncing saved items"),
        }
    }

    let report = SyncReport {
        playlists: all.len(),
        tracks: all.iter().map(|p| p.tracks.len()).sum(),
        unchanged: 0,
        quota_units: 0,
        liked: liked_count,
        liked_source: LikedSource::YouTubeMusic,
        via_session: true,
    };
    tokio::task::spawn_blocking(move || library.replace_synced(&all)).await??;
    Ok(report)
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
        via_session: false,
    };
    tokio::task::spawn_blocking(move || library.replace_synced(&all)).await??;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(via_session: bool) -> SyncReport {
        SyncReport {
            playlists: 5,
            tracks: 812,
            unchanged: 3,
            quota_units: 21,
            liked: 120,
            liked_source: LikedSource::YouTubeMusic,
            via_session,
        }
    }

    #[test]
    fn summaries_say_what_the_sync_cost() {
        assert_eq!(
            report(true).summary(Lang::En),
            "Synced 5 playlists, 812 tracks (no API quota) · 120 liked (YouTube Music)"
        );
        assert_eq!(
            report(false).summary(Lang::En),
            "Synced 5 playlists, 812 tracks (3 unchanged, 21 API units) · 120 liked (YouTube Music)"
        );
    }
    /// The signed-in library, read-only on the account. Needs the user's own
    /// sign-in, so it's opt-in:
    /// `YTM_COOKIES=<data_dir>/youtube-cookies.txt cargo test -- --ignored live_session_sync`
    #[tokio::test]
    #[ignore]
    async fn live_session_sync() {
        use crate::{
            catalog::ItemKind,
            youtube_session::{InMemory, YouTubeSession, parse_cookies_txt},
        };

        let Ok(path) = std::env::var("YTM_COOKIES") else {
            return;
        };
        let cookies = parse_cookies_txt(&std::fs::read_to_string(path).unwrap());
        let file = std::env::temp_dir().join(format!("ytm-live-sync-{}.txt", std::process::id()));
        let session = YouTubeSession::load(InMemory::default(), file.clone());
        session.sign_in(cookies).unwrap();
        let catalog = Catalog::new(reqwest::Client::new()).with_session(session);
        let library = Arc::new(Library::open_in_memory().unwrap());

        let report = sync_with_session(&catalog, library.clone()).await.unwrap();
        std::fs::remove_file(&file).ok();
        assert!(
            report.liked > 0 && report.playlists > 1 && report.quota_units == 0,
            "{}",
            report.summary(Lang::En)
        );
        assert_eq!(
            library.tracks(LIKED_PLAYLIST_ID).unwrap().len(),
            report.liked
        );
        assert!(!library.saved(ItemKind::Artist).unwrap().is_empty());
        // A playlist entry knows its id, so it can be removed again.
        let playlist = library
            .playlists()
            .unwrap()
            .into_iter()
            .find(|p| p.id != LIKED_PLAYLIST_ID && p.item_count > 0)
            .expect("a playlist of the account's own");
        let track = library.tracks(&playlist.id).unwrap()[0].clone();
        assert!(
            library
                .item_id(&playlist.id, &track.video_id)
                .unwrap()
                .is_some(),
            "{}: {}",
            playlist.title,
            track.title
        );
    }
}

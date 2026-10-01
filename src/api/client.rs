//! Typed YouTube Data API v3 client. Every `*.list` page costs 1 quota unit
//! (10,000/day); [`YouTubeClient::units_used`] counts them for this process.

use std::sync::{
    Arc,
    atomic::{AtomicU32, Ordering},
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, de::DeserializeOwned};

use super::{
    auth::Auth,
    models::{Playlist, Track, clean_artist, parse_iso_duration},
};

const BASE: &str = "https://www.googleapis.com/youtube/v3";
/// YouTube's "Music" video category.
const MUSIC_CATEGORY: &str = "10";

pub struct YouTubeClient {
    http: reqwest::Client,
    auth: Arc<Auth>,
    units: AtomicU32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Page<T> {
    #[serde(default = "Vec::new")]
    items: Vec<T>,
    next_page_token: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PlaylistDto {
    id: String,
    snippet: PlaylistSnippet,
    content_details: Option<PlaylistContent>,
}
#[derive(Deserialize)]
struct PlaylistSnippet {
    title: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PlaylistContent {
    item_count: Option<u32>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PlaylistItemDto {
    snippet: ItemSnippet,
    content_details: ItemContent,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ItemSnippet {
    title: String,
    video_owner_channel_title: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ItemContent {
    video_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct VideoDto {
    id: String,
    snippet: VideoSnippet,
    content_details: Option<VideoContent>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct VideoSnippet {
    title: String,
    channel_title: String,
    category_id: Option<String>,
}
#[derive(Deserialize)]
struct VideoContent {
    duration: Option<String>,
}

impl YouTubeClient {
    pub fn new(http: reqwest::Client, auth: Arc<Auth>) -> Self {
        Self {
            http,
            auth,
            units: AtomicU32::new(0),
        }
    }

    pub fn units_used(&self) -> u32 {
        self.units.load(Ordering::Relaxed)
    }

    /// Fetches every page of a list endpoint.
    async fn list_all<T: DeserializeOwned>(
        &self,
        endpoint: &str,
        query: &[(&str, &str)],
    ) -> Result<Vec<T>> {
        let mut items = Vec::new();
        let mut page_token: Option<String> = None;
        loop {
            let token = self.auth.access_token().await?;
            let mut request = self
                .http
                .get(format!("{BASE}/{endpoint}"))
                .bearer_auth(token)
                .query(query)
                .query(&[("maxResults", "50")]);
            if let Some(page) = &page_token {
                request = request.query(&[("pageToken", page)]);
            }
            let response = request.send().await?;
            self.units.fetch_add(1, Ordering::Relaxed);

            let status = response.status();
            if !status.is_success() {
                let body = response.text().await.unwrap_or_default();
                bail!(
                    "YouTube API {endpoint}: {status}: {}",
                    api_error_message(&body)
                );
            }
            let page: Page<T> = response
                .json()
                .await
                .with_context(|| format!("decoding {endpoint} response"))?;
            items.extend(page.items);
            match page.next_page_token {
                Some(next) => page_token = Some(next),
                None => return Ok(items),
            }
        }
    }

    pub async fn my_playlists(&self) -> Result<Vec<Playlist>> {
        let dtos: Vec<PlaylistDto> = self
            .list_all(
                "playlists",
                &[("part", "snippet,contentDetails"), ("mine", "true")],
            )
            .await?;
        Ok(dtos
            .into_iter()
            .map(|p| Playlist {
                id: p.id,
                title: p.snippet.title,
                item_count: p.content_details.and_then(|c| c.item_count).unwrap_or(0),
            })
            .collect())
    }

    pub async fn playlist_tracks(&self, playlist_id: &str) -> Result<Vec<Track>> {
        let dtos: Vec<PlaylistItemDto> = self
            .list_all(
                "playlistItems",
                &[
                    ("part", "snippet,contentDetails"),
                    ("playlistId", playlist_id),
                ],
            )
            .await?;
        Ok(dtos
            .into_iter()
            // Deleted/private entries have no owner channel and can't be played.
            .filter_map(|i| {
                let artist = i.snippet.video_owner_channel_title?;
                Some(Track {
                    video_id: i.content_details.video_id,
                    title: i.snippet.title,
                    artist: clean_artist(&artist),
                    duration_secs: None,
                })
            })
            .collect())
    }

    /// Liked videos; with `music_only`, only the "Music" category, which
    /// approximates YouTube Music's "Liked music" (the API has no such list).
    pub async fn liked_tracks(&self, music_only: bool) -> Result<Vec<Track>> {
        let dtos: Vec<VideoDto> = self
            .list_all(
                "videos",
                &[("part", "snippet,contentDetails"), ("myRating", "like")],
            )
            .await?;
        Ok(dtos
            .into_iter()
            .filter(|v| !music_only || v.snippet.category_id.as_deref() == Some(MUSIC_CATEGORY))
            .map(|v| Track {
                video_id: v.id,
                title: v.snippet.title,
                artist: clean_artist(&v.snippet.channel_title),
                duration_secs: v
                    .content_details
                    .and_then(|c| c.duration)
                    .and_then(|d| parse_iso_duration(&d)),
            })
            .collect())
    }
}

/// Pulls `error.message` out of a Google API error body.
fn api_error_message(body: &str) -> String {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v["error"]["message"].as_str().map(str::to_owned))
        .unwrap_or_else(|| body.chars().take(200).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_playlist_items_page() {
        let json = r#"{
          "nextPageToken": "abc",
          "items": [
            {"snippet": {"title": "Song", "videoOwnerChannelTitle": "Band - Topic"},
             "contentDetails": {"videoId": "dQw4w9WgXcQ"}},
            {"snippet": {"title": "Deleted video"},
             "contentDetails": {"videoId": "xxxxxxxxxxx"}}
          ]
        }"#;
        let page: Page<PlaylistItemDto> = serde_json::from_str(json).unwrap();
        assert_eq!(page.next_page_token.as_deref(), Some("abc"));
        assert_eq!(page.items.len(), 2);
        assert!(page.items[1].snippet.video_owner_channel_title.is_none());
    }

    #[test]
    fn extracts_api_error_message() {
        let body = r#"{"error":{"code":403,"message":"quotaExceeded"}}"#;
        assert_eq!(api_error_message(body), "quotaExceeded");
    }
}

//! Typed YouTube Data API v3 client. Quota (10,000 units/day): each `*.list`
//! page costs 1 unit, writes (rate, playlist and subscription changes) 50
//! each, `search.list` 100;
//! [`YouTubeClient::units_used`] counts them for this process.

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
use crate::catalog::{Item, ItemKind};

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
    etag: Option<String>,
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
    #[serde(default)]
    id: String,
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
struct SearchItemDto {
    id: SearchId,
    snippet: SearchSnippet,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SearchId {
    video_id: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SearchSnippet {
    title: String,
    channel_title: String,
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

            let response = check(response, endpoint).await?;
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
                etag: p.etag,
                title: p.snippet.title,
                item_count: p.content_details.and_then(|c| c.item_count).unwrap_or(0),
            })
            .collect())
    }

    pub async fn playlist_tracks(&self, playlist_id: &str) -> Result<Vec<Track>> {
        Ok(self.playlist_entries(playlist_id).await?.0)
    }

    /// A playlist's tracks and, for each, its playlist item id (what
    /// `playlistItems.delete` needs).
    pub async fn playlist_entries(&self, playlist_id: &str) -> Result<(Vec<Track>, Vec<String>)> {
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
                let track = Track {
                    video_id: i.content_details.video_id.into(),
                    title: i.snippet.title.into(),
                    artist: clean_artist(&artist),
                    duration_secs: None,
                };
                Some((track, i.id))
            })
            .unzip())
    }

    /// The user's liked songs.
    ///
    /// Tries YouTube Music's own "Liked music" playlist (`LM`) first — exact,
    /// but not officially documented for the Data API. Falls back to liked
    /// YouTube videos, optionally filtered to the "Music" category.
    pub async fn liked_tracks(&self, music_only: bool) -> Result<Liked> {
        match self.playlist_tracks(LIKED_MUSIC_PLAYLIST).await {
            Ok(mut tracks) if !tracks.is_empty() => {
                self.fill_durations(&mut tracks).await;
                return Ok(Liked {
                    tracks,
                    source: LikedSource::YouTubeMusic,
                });
            }
            Ok(_) => tracing::info!("Liked music (LM) is empty; using liked videos"),
            Err(err) => tracing::info!(%err, "Liked music (LM) unavailable; using liked videos"),
        }

        let dtos: Vec<VideoDto> = self
            .list_all(
                "videos",
                &[("part", "snippet,contentDetails"), ("myRating", "like")],
            )
            .await?;
        let total = dtos.len();
        let tracks = dtos
            .into_iter()
            .filter(|v| !music_only || v.snippet.category_id.as_deref() == Some(MUSIC_CATEGORY))
            .map(|v| Track {
                video_id: v.id.into(),
                title: v.snippet.title.into(),
                artist: clean_artist(&v.snippet.channel_title),
                duration_secs: v
                    .content_details
                    .and_then(|c| c.duration)
                    .and_then(|d| parse_iso_duration(&d)),
            })
            .collect();
        Ok(Liked {
            tracks,
            source: LikedSource::LikedVideos { total, music_only },
        })
    }

    /// Fills `duration_secs` from `videos.list` (1 unit per 50 tracks).
    /// Best effort: durations are cosmetic, so errors are only logged.
    async fn fill_durations(&self, tracks: &mut [Track]) {
        for chunk in tracks.chunks_mut(50) {
            let ids = chunk
                .iter()
                .map(|t| &*t.video_id)
                .collect::<Vec<_>>()
                .join(",");
            let result: Result<Vec<VideoDto>> = async {
                // `maxResults` can't be combined with `id`, so not via list_all.
                let response = self
                    .http
                    .get(format!("{BASE}/videos"))
                    .bearer_auth(self.auth.access_token().await?)
                    .query(&[("part", "snippet,contentDetails"), ("id", ids.as_str())])
                    .send()
                    .await?;
                self.units.fetch_add(1, Ordering::Relaxed);
                let page: Page<VideoDto> = check(response, "videos").await?.json().await?;
                Ok(page.items)
            }
            .await;
            match result {
                Ok(videos) => {
                    for track in chunk.iter_mut() {
                        track.duration_secs = videos
                            .iter()
                            .find(|v| *v.id == *track.video_id)
                            .and_then(|v| v.content_details.as_ref()?.duration.as_deref())
                            .and_then(parse_iso_duration);
                    }
                }
                Err(err) => tracing::warn!(%err, "fetching durations"),
            }
        }
    }
}

impl YouTubeClient {
    /// Searches music videos. 100 units (+1 for durations), so only on an
    /// explicit request, never as-you-type.
    pub async fn search(&self, query: &str, max: u8) -> Result<Vec<Track>> {
        let max = max.clamp(1, 50).to_string();
        let response = self
            .http
            .get(format!("{BASE}/search"))
            .bearer_auth(self.auth.access_token().await?)
            .query(&[
                ("part", "snippet"),
                ("type", "video"),
                ("videoCategoryId", MUSIC_CATEGORY),
                ("maxResults", max.as_str()),
                ("q", query),
            ])
            .send()
            .await?;
        self.units.fetch_add(100, Ordering::Relaxed);
        let page: Page<SearchItemDto> = check(response, "search").await?.json().await?;
        let mut tracks: Vec<Track> = page
            .items
            .into_iter()
            .filter_map(|i| {
                Some(Track {
                    video_id: i.id.video_id?.into(),
                    title: unescape_html(&i.snippet.title).into(),
                    artist: clean_artist(&unescape_html(&i.snippet.channel_title)),
                    duration_secs: None,
                })
            })
            .collect();
        self.fill_durations(&mut tracks).await;
        Ok(tracks)
    }
}

/// `search.list` snippets come HTML-escaped (`&amp;`, `&#39;`, `&quot;`).
fn unescape_html(s: &str) -> String {
    if !s.contains('&') {
        return s.to_owned();
    }
    s.replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

/// YouTube Music's "Liked music" playlist id.
const LIKED_MUSIC_PLAYLIST: &str = "LM";

pub struct Liked {
    pub tracks: Vec<Track>,
    pub source: LikedSource,
}

#[derive(Debug, Clone, Copy)]
pub enum LikedSource {
    /// The exact YouTube Music "Liked music" list.
    YouTubeMusic,
    /// Liked YouTube videos (`total` before filtering).
    LikedVideos { total: usize, music_only: bool },
}

impl YouTubeClient {
    /// Likes (`true`) or removes the rating from (`false`) a video. 50 units.
    pub async fn rate(&self, video_id: &str, like: bool) -> Result<()> {
        self.set_rating(video_id, if like { Rating::Like } else { Rating::None })
            .await
    }

    /// Sets a video's rating. 50 units.
    pub async fn set_rating(&self, video_id: &str, rating: Rating) -> Result<()> {
        let rating = match rating {
            Rating::Like => "like",
            Rating::Dislike => "dislike",
            Rating::None => "none",
        };
        let response = self
            .http
            .post(format!("{BASE}/videos/rate"))
            .bearer_auth(self.auth.access_token().await?)
            .query(&[("id", video_id), ("rating", rating)])
            .header(reqwest::header::CONTENT_LENGTH, 0)
            .send()
            .await?;
        self.units.fetch_add(50, Ordering::Relaxed);
        check(response, "videos.rate").await?;
        Ok(())
    }

    /// Appends a video to one of the user's playlists; returns the new
    /// playlist item id. 50 units.
    pub async fn add_to_playlist(&self, playlist_id: &str, video_id: &str) -> Result<String> {
        let body = serde_json::json!({
            "snippet": {
                "playlistId": playlist_id,
                "resourceId": { "kind": "youtube#video", "videoId": video_id },
            }
        });
        let response = self
            .http
            .post(format!("{BASE}/playlistItems"))
            .bearer_auth(self.auth.access_token().await?)
            .query(&[("part", "snippet")])
            .json(&body)
            .send()
            .await?;
        self.units.fetch_add(50, Ordering::Relaxed);
        let created: Created = check(response, "playlistItems.insert")
            .await?
            .json()
            .await?;
        Ok(created.id)
    }

    /// Removes a playlist item (id from [`playlist_entries`] or
    /// [`add_to_playlist`]). 50 units.
    ///
    /// [`playlist_entries`]: Self::playlist_entries
    /// [`add_to_playlist`]: Self::add_to_playlist
    pub async fn remove_from_playlist(&self, item_id: &str) -> Result<()> {
        let response = self
            .http
            .delete(format!("{BASE}/playlistItems"))
            .bearer_auth(self.auth.access_token().await?)
            .query(&[("id", item_id)])
            .send()
            .await?;
        self.units.fetch_add(50, Ordering::Relaxed);
        check(response, "playlistItems.delete").await?;
        Ok(())
    }

    /// Creates a private playlist; returns its id. 50 units.
    pub async fn create_playlist(&self, title: &str) -> Result<String> {
        let body = serde_json::json!({
            "snippet": { "title": title },
            "status": { "privacyStatus": "private" },
        });
        let response = self
            .http
            .post(format!("{BASE}/playlists"))
            .bearer_auth(self.auth.access_token().await?)
            .query(&[("part", "snippet,status")])
            .json(&body)
            .send()
            .await?;
        self.units.fetch_add(50, Ordering::Relaxed);
        let created: Created = check(response, "playlists.insert").await?.json().await?;
        Ok(created.id)
    }

    /// Renames a playlist. 50 units.
    pub async fn rename_playlist(&self, playlist_id: &str, title: &str) -> Result<()> {
        let body = serde_json::json!({ "id": playlist_id, "snippet": { "title": title } });
        let response = self
            .http
            .put(format!("{BASE}/playlists"))
            .bearer_auth(self.auth.access_token().await?)
            .query(&[("part", "snippet")])
            .json(&body)
            .send()
            .await?;
        self.units.fetch_add(50, Ordering::Relaxed);
        check(response, "playlists.update").await?;
        Ok(())
    }

    /// Deletes a playlist. 50 units.
    pub async fn delete_playlist(&self, playlist_id: &str) -> Result<()> {
        let response = self
            .http
            .delete(format!("{BASE}/playlists"))
            .bearer_auth(self.auth.access_token().await?)
            .query(&[("id", playlist_id)])
            .send()
            .await?;
        self.units.fetch_add(50, Ordering::Relaxed);
        check(response, "playlists.delete").await?;
        Ok(())
    }

    /// Channels the user subscribed to, as artists. 1 unit per 50.
    pub async fn subscriptions(&self) -> Result<Vec<Item>> {
        let dtos: Vec<SubscriptionDto> = self
            .list_all("subscriptions", &[("part", "snippet"), ("mine", "true")])
            .await?;
        Ok(dtos
            .into_iter()
            .map(|s| Item {
                kind: ItemKind::Artist,
                id: s.snippet.resource_id.channel_id.into(),
                title: s.snippet.title.into(),
                subtitle: "Artist".into(),
                thumbnail: s
                    .snippet
                    .thumbnails
                    .and_then(|t| t.medium.or(t.default))
                    .map(|t| t.url.into()),
                track: None,
            })
            .collect())
    }

    /// Subscribes to a channel. 50 units.
    pub async fn subscribe(&self, channel_id: &str) -> Result<()> {
        let body = serde_json::json!({
            "snippet": { "resourceId": { "kind": "youtube#channel", "channelId": channel_id } }
        });
        let response = self
            .http
            .post(format!("{BASE}/subscriptions"))
            .bearer_auth(self.auth.access_token().await?)
            .query(&[("part", "snippet")])
            .json(&body)
            .send()
            .await?;
        self.units.fetch_add(50, Ordering::Relaxed);
        check(response, "subscriptions.insert").await?;
        Ok(())
    }

    /// Unsubscribes from a channel (finds the subscription first). 51 units.
    pub async fn unsubscribe(&self, channel_id: &str) -> Result<()> {
        let page: Page<IdOnly> = check(
            self.http
                .get(format!("{BASE}/subscriptions"))
                .bearer_auth(self.auth.access_token().await?)
                .query(&[
                    ("part", "id"),
                    ("mine", "true"),
                    ("forChannelId", channel_id),
                ])
                .send()
                .await?,
            "subscriptions.list",
        )
        .await?
        .json()
        .await?;
        self.units.fetch_add(1, Ordering::Relaxed);
        for sub in page.items {
            let response = self
                .http
                .delete(format!("{BASE}/subscriptions"))
                .bearer_auth(self.auth.access_token().await?)
                .query(&[("id", sub.id.as_str())])
                .send()
                .await?;
            self.units.fetch_add(50, Ordering::Relaxed);
            check(response, "subscriptions.delete").await?;
        }
        Ok(())
    }
}

/// A video rating.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rating {
    Like,
    Dislike,
    None,
}

#[derive(Deserialize)]
struct Created {
    id: String,
}

#[derive(Deserialize)]
struct IdOnly {
    id: String,
}

#[derive(Deserialize)]
struct SubscriptionDto {
    snippet: SubscriptionSnippet,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SubscriptionSnippet {
    title: String,
    resource_id: ResourceId,
    thumbnails: Option<Thumbnails>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResourceId {
    channel_id: String,
}
#[derive(Deserialize)]
struct Thumbnails {
    default: Option<Thumb>,
    medium: Option<Thumb>,
}
#[derive(Deserialize)]
struct Thumb {
    url: String,
}

/// Turns API errors into readable messages, with a hint for missing scope.
async fn check(response: reqwest::Response, endpoint: &str) -> Result<reqwest::Response> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let body = response.text().await.unwrap_or_default();
    if body.contains("insufficientPermissions") || body.contains("SCOPE_INSUFFICIENT") {
        bail!(
            "no permission to change your library — {} to allow it",
            super::auth::SIGN_IN_AGAIN
        );
    }
    bail!(
        "YouTube API {endpoint}: {status}: {}",
        api_error_message(&body)
    )
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
    fn decodes_search_page() {
        let json = r#"{"items": [
            {"id": {"kind": "youtube#video", "videoId": "fJ9rUzIMcZQ"},
             "snippet": {"title": "Queen &amp; friends &#39;live&#39;", "channelTitle": "Queen - Topic"}},
            {"id": {"kind": "youtube#channel"}, "snippet": {"title": "x", "channelTitle": "x"}}
        ]}"#;
        let page: Page<SearchItemDto> = serde_json::from_str(json).unwrap();
        assert_eq!(page.items.len(), 2);
        assert!(page.items[1].id.video_id.is_none());
        assert_eq!(
            unescape_html(&page.items[0].snippet.title),
            "Queen & friends 'live'"
        );
    }

    #[test]
    fn extracts_api_error_message() {
        let body = r#"{"error":{"code":403,"message":"quotaExceeded"}}"#;
        assert_eq!(api_error_message(body), "quotaExceeded");
    }
}

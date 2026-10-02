//! YouTube Music catalog: search by category, artist / album / playlist
//! pages, the home page, radio ("Up next") and the lyrics of a song.
//!
//! Uses the web client's own endpoints at music.youtube.com (InnerTube, the
//! API the YouTube Music site and yt-dlp use). Unofficial: no quota and no
//! sign-in, but its JSON shapes can change, so parsing is lenient — unknown
//! renderers are skipped, never an error. Responses are 50–600 KB of JSON,
//! parsed and dropped per request; nothing is cached here.

use std::sync::Arc;

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

use crate::api::models::{Track, clean_artist};

const BASE: &str = "https://music.youtube.com/youtubei/v1";
const CLIENT_NAME: &str = "WEB_REMIX";
/// Any recent web client version works; YouTube accepts old ones for years.
const CLIENT_VERSION: &str = "1.20260901.01.00";
/// The player endpoint's loudness data is only given to this client.
const IOS_CLIENT: &str = "IOS";
const IOS_VERSION: &str = "20.10.4";
/// Continuation pages fetched for long playlists (~100 tracks each).
const MAX_PAGES: usize = 10;

/// Search filters (protobuf params the web client sends for each tab).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SearchKind {
    Songs,
    Albums,
    Artists,
    Playlists,
}

impl SearchKind {
    pub const ALL: [Self; 4] = [Self::Songs, Self::Albums, Self::Artists, Self::Playlists];

    fn params(self) -> &'static str {
        match self {
            Self::Songs => "EgWKAQIIAWoMEA4QChADEAQQCRAF",
            Self::Albums => "EgWKAQIYAWoMEA4QChADEAQQCRAF",
            Self::Artists => "EgWKAQIgAWoMEA4QChADEAQQCRAF",
            Self::Playlists => "EgWKAQIoAWoMEA4QChADEAQQCRAF",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Songs => "Songs",
            Self::Albums => "Albums",
            Self::Artists => "Artists",
            Self::Playlists => "Playlists",
        }
    }
}

/// What a catalog entry opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemKind {
    Song,
    Album,
    Artist,
    Playlist,
}

/// A search result or a card on a page.
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub kind: ItemKind,
    /// Video id for songs, browse id (`MPREb_…`, `UC…`, `VL…`) otherwise.
    pub id: Arc<str>,
    pub title: Arc<str>,
    /// "Album • ABBA • 1992", "Artist • 290M monthly audience", …
    pub subtitle: Arc<str>,
    /// Square art URL (≈226 px) when the catalog gives one.
    pub thumbnail: Option<Arc<str>>,
    /// Songs only: playable.
    pub track: Option<Track>,
}

/// A titled row of cards (home page, artist page sections).
#[derive(Debug, Clone, PartialEq)]
pub struct Shelf {
    pub title: String,
    pub items: Vec<Item>,
}

/// An album, artist or playlist page.
#[derive(Debug, Clone, PartialEq)]
pub struct Page {
    pub kind: ItemKind,
    pub id: String,
    pub title: String,
    /// "Album • ABBA • 1992", the artist's audience, …
    pub subtitle: String,
    pub thumbnail: Option<String>,
    /// Songs on the page (album tracks, artist's top songs, playlist).
    pub tracks: Vec<Track>,
    /// Other sections (albums, singles, similar artists, …).
    pub shelves: Vec<Shelf>,
    /// Artists: the YouTube channel id to subscribe to.
    pub channel_id: Option<String>,
}

/// Lyrics text, with the provider's credit line.
#[derive(Debug, Clone, PartialEq)]
pub struct Lyrics {
    pub text: String,
    pub source: String,
}

#[derive(Clone)]
pub struct Catalog {
    http: reqwest::Client,
}

impl Catalog {
    pub fn new(http: reqwest::Client) -> Self {
        Self { http }
    }

    async fn call(&self, endpoint: &str, body: Value) -> Result<Value> {
        self.call_as(CLIENT_NAME, CLIENT_VERSION, endpoint, body)
            .await
    }

    async fn call_as(
        &self,
        client: &str,
        version: &str,
        endpoint: &str,
        mut body: Value,
    ) -> Result<Value> {
        body["context"] = json!({
            "client": { "clientName": client, "clientVersion": version, "hl": "en", "gl": "US" }
        });
        let response = self
            .http
            .post(format!("{BASE}/{endpoint}?prettyPrint=false"))
            .header("Origin", "https://music.youtube.com")
            .json(&body)
            .send()
            .await
            .with_context(|| format!("YouTube Music {endpoint}"))?;
        let status = response.status();
        if !status.is_success() {
            bail!("YouTube Music {endpoint}: HTTP {status}");
        }
        Ok(response.json().await?)
    }

    pub async fn search(&self, query: &str, kind: SearchKind) -> Result<Vec<Item>> {
        let data = self
            .call("search", json!({ "query": query, "params": kind.params() }))
            .await?;
        Ok(parse_search(&data, kind))
    }

    /// Album (`MPREb_…`), artist (`UC…`) or playlist (`VL…` / `PL…`) page.
    pub async fn page(&self, browse_id: &str) -> Result<Page> {
        let id = match browse_id {
            id if id.starts_with("PL") || id.starts_with("OLAK") || id.starts_with("RD") => {
                format!("VL{id}")
            }
            id => id.to_owned(),
        };
        let data = self.call("browse", json!({ "browseId": id })).await?;
        let mut page = parse_page(&id, &data);
        // Long playlists come in pages.
        let mut token = continuation(&data);
        let mut pages = 1;
        while let (Some(t), ItemKind::Playlist) = (token.take(), page.kind) {
            if pages >= MAX_PAGES {
                break;
            }
            pages += 1;
            let more = self.call("browse", json!({ "continuation": t })).await?;
            let before = page.tracks.len();
            page.tracks.extend(songs_in(&more, None));
            if page.tracks.len() == before {
                break;
            }
            token = continuation(&more);
        }
        // An artist page lists 5 top songs; its "Show all" playlist has
        // them all (with durations), so Play / Shuffle get the full set.
        if page.kind == ItemKind::Artist
            && let Some(all) = top_songs_playlist(&data)
        {
            match Box::pin(self.page(&all)).await {
                Ok(list) if list.tracks.len() > page.tracks.len() => page.tracks = list.tracks,
                Ok(_) => {}
                Err(err) => tracing::debug!(%err, "artist's top songs"),
            }
        }
        Ok(page)
    }

    /// The home page's shelves (not personalised: no sign-in here).
    pub async fn home(&self) -> Result<Vec<Shelf>> {
        let data = self
            .call("browse", json!({ "browseId": "FEmusic_home" }))
            .await?;
        Ok(parse_shelves(&data))
    }

    /// The song's radio: what YouTube Music plays after it (≈50 songs,
    /// the song itself first).
    pub async fn radio(&self, video_id: &str) -> Result<Vec<Track>> {
        let data = self.next(video_id).await?;
        Ok(parse_radio(&data))
    }

    /// "Related" tab: similar songs, artists and albums.
    pub async fn related(&self, video_id: &str) -> Result<Vec<Shelf>> {
        let next = self.next(video_id).await?;
        let Some(id) = tab_browse_id(&next, "Related") else {
            return Ok(Vec::new());
        };
        let data = self.call("browse", json!({ "browseId": id })).await?;
        Ok(parse_shelves(&data))
    }

    /// Plain lyrics from YouTube Music's provider, when it has them.
    pub async fn lyrics(&self, video_id: &str) -> Result<Option<Lyrics>> {
        let next = self.next(video_id).await?;
        let Some(id) = tab_browse_id(&next, "Lyrics") else {
            return Ok(None);
        };
        let data = self.call("browse", json!({ "browseId": id })).await?;
        Ok(parse_lyrics(&data))
    }

    /// Loudness of the AAC stream (itag 140) relative to YouTube's target,
    /// in dB (positive: louder than the target; YouTube turns it down).
    /// Only the iOS client's player response carries it.
    pub async fn loudness_db(&self, video_id: &str) -> Result<Option<f32>> {
        let data = self
            .call_as(
                IOS_CLIENT,
                IOS_VERSION,
                "player",
                json!({ "videoId": video_id }),
            )
            .await?;
        Ok(parse_loudness(&data))
    }

    /// The page id of the song's (first) artist, as YouTube Music links
    /// it under the song.
    pub async fn artist_id(&self, video_id: &str) -> Result<Option<String>> {
        let data = self.next(video_id).await?;
        let items = find_all(&data, "playlistPanelVideoRenderer");
        let song = items
            .iter()
            .find(|v| v.get("videoId").and_then(Value::as_str) == Some(video_id))
            .or(items.first());
        Ok(song
            .and_then(|v| v.pointer("/longBylineText/runs"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|run| run.pointer("/navigationEndpoint/browseEndpoint/browseId"))
            .filter_map(Value::as_str)
            .find(|id| id.starts_with("UC"))
            .map(str::to_owned))
    }

    async fn next(&self, video_id: &str) -> Result<Value> {
        self.call(
            "next",
            json!({
                "videoId": video_id,
                "playlistId": format!("RDAMVM{video_id}"),
                "isAudioOnly": true,
            }),
        )
        .await
    }
}

// --- parsing ---------------------------------------------------------------------
//
// YouTube Music's JSON is a tree of "renderers". Helpers find them anywhere
// below a node, so wrapper changes (tabs, sections) don't break parsing.

/// All values under `key`, depth-first, without descending into matches.
fn find<'a>(v: &'a Value, key: &str, out: &mut Vec<&'a Value>) {
    match v {
        Value::Object(map) => {
            for (k, child) in map {
                if k == key {
                    out.push(child);
                } else {
                    find(child, key, out);
                }
            }
        }
        Value::Array(items) => items.iter().for_each(|c| find(c, key, out)),
        _ => {}
    }
}

fn find_all<'a>(v: &'a Value, key: &str) -> Vec<&'a Value> {
    let mut out = Vec::new();
    find(v, key, &mut out);
    out
}

fn find_first<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    find_all(v, key).into_iter().next()
}

/// Concatenated `runs[].text` (or `simpleText`).
fn text(v: Option<&Value>) -> String {
    let Some(v) = v else {
        return String::new();
    };
    if let Some(s) = v.get("simpleText").and_then(Value::as_str) {
        return s.to_owned();
    }
    v.get("runs")
        .and_then(Value::as_array)
        .map(|runs| {
            runs.iter()
                .filter_map(|r| r.get("text").and_then(Value::as_str))
                .collect()
        })
        .unwrap_or_default()
}

fn runs(v: Option<&Value>) -> &[Value] {
    v.and_then(|v| v.get("runs"))
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

fn page_type(endpoint: &Value) -> Option<&str> {
    endpoint
        .pointer("/browseEndpoint/browseEndpointContextSupportedConfigs/browseEndpointContextMusicConfig/pageType")
        .and_then(Value::as_str)
}

fn kind_of(page_type: &str) -> Option<ItemKind> {
    Some(match page_type {
        "MUSIC_PAGE_TYPE_ALBUM" | "MUSIC_PAGE_TYPE_AUDIOBOOK" => ItemKind::Album,
        "MUSIC_PAGE_TYPE_ARTIST" | "MUSIC_PAGE_TYPE_USER_CHANNEL" => ItemKind::Artist,
        "MUSIC_PAGE_TYPE_PLAYLIST" => ItemKind::Playlist,
        _ => return None,
    })
}

/// "3:52" / "1:02:03" → seconds.
fn parse_duration(s: &str) -> Option<u32> {
    let mut total = 0u32;
    let mut parts = 0;
    for part in s.trim().split(':') {
        total = total.checked_mul(60)?.checked_add(part.parse().ok()?)?;
        parts += 1;
    }
    (parts >= 2).then_some(total)
}

/// The thumbnail ≥ 200 px wide (else the largest), as an URL.
fn thumbnail(v: &Value) -> Option<Arc<str>> {
    let thumbs = find_first(v, "thumbnails")?.as_array()?;
    let pick = thumbs
        .iter()
        .find(|t| t.get("width").and_then(Value::as_u64).unwrap_or(0) >= 200)
        .or_else(|| thumbs.last())?;
    pick.get("url").and_then(Value::as_str).map(Arc::from)
}

/// `musicResponsiveListItemRenderer`: a row in search results, shelves and
/// track lists. `album_artist` fills in album pages' artist-less rows.
fn list_item(r: &Value, album_artist: Option<&str>) -> Option<Item> {
    let columns = r.get("flexColumns")?.as_array()?;
    let column = |i: usize| {
        columns
            .get(i)
            .and_then(|c| c.pointer("/musicResponsiveListItemFlexColumnRenderer/text"))
    };
    let title = text(column(0));
    if title.is_empty() {
        return None;
    }
    let second = runs(column(1));
    let subtitle = text(column(1));
    let thumbnail = r.get("thumbnail").and_then(thumbnail);

    let video_id = r
        .pointer("/playlistItemData/videoId")
        .or_else(|| find_first(r, "watchEndpoint").and_then(|w| w.get("videoId")))
        .and_then(Value::as_str);
    let browse = r
        .get("navigationEndpoint")
        .filter(|e| e.get("browseEndpoint").is_some());
    match (video_id, browse) {
        (_, Some(endpoint)) => {
            let kind = page_type(endpoint).and_then(kind_of)?;
            let id = endpoint.pointer("/browseEndpoint/browseId")?.as_str()?;
            Some(Item {
                kind,
                id: id.into(),
                title: title.into(),
                subtitle: subtitle.into(),
                thumbnail,
                track: None,
            })
        }
        (Some(video_id), None) => {
            // Artists are the runs linking to artist pages; else the first run.
            let artists: Vec<&str> = second
                .iter()
                .filter(|run| {
                    run.get("navigationEndpoint")
                        .and_then(page_type)
                        .is_some_and(|t| kind_of(t) == Some(ItemKind::Artist))
                })
                .filter_map(|run| run.get("text").and_then(Value::as_str))
                .collect();
            let artist = if !artists.is_empty() {
                artists.join(", ")
            } else if let Some(artist) = album_artist {
                artist.to_owned()
            } else {
                second
                    .first()
                    .and_then(|run| run.get("text"))
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned()
            };
            let duration = r
                .get("fixedColumns")
                .and_then(|c| c.get(0))
                .map(|c| text(c.pointer("/musicResponsiveListItemFixedColumnRenderer/text")))
                .and_then(|s| parse_duration(&s))
                .or_else(|| {
                    second
                        .iter()
                        .rev()
                        .filter_map(|run| run.get("text").and_then(Value::as_str))
                        .find_map(parse_duration)
                });
            let track = Track {
                video_id: video_id.into(),
                title: title.as_str().into(),
                artist: clean_artist(&artist),
                duration_secs: duration,
            };
            Some(Item {
                kind: ItemKind::Song,
                id: video_id.into(),
                title: title.into(),
                subtitle: subtitle.into(),
                thumbnail,
                track: Some(track),
            })
        }
        (None, None) => None,
    }
}

/// `musicTwoRowItemRenderer`: a card (album, playlist, artist, song).
fn card(r: &Value) -> Option<Item> {
    let title = text(r.get("title"));
    let subtitle = text(r.get("subtitle"));
    let endpoint = r.get("navigationEndpoint")?;
    let thumbnail = r.get("thumbnailRenderer").and_then(thumbnail);
    if let Some(video_id) = endpoint
        .pointer("/watchEndpoint/videoId")
        .and_then(Value::as_str)
    {
        let artist = runs(r.get("subtitle"))
            .iter()
            .find(|run| run.get("navigationEndpoint").is_some())
            .and_then(|run| run.get("text"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        return Some(Item {
            kind: ItemKind::Song,
            id: video_id.into(),
            track: Some(Track {
                video_id: video_id.into(),
                title: title.as_str().into(),
                artist: clean_artist(artist),
                duration_secs: None,
            }),
            title: title.into(),
            subtitle: subtitle.into(),
            thumbnail,
        });
    }
    let kind = page_type(endpoint).and_then(kind_of)?;
    let id = endpoint.pointer("/browseEndpoint/browseId")?.as_str()?;
    Some(Item {
        kind,
        id: id.into(),
        title: title.into(),
        subtitle: subtitle.into(),
        thumbnail,
        track: None,
    })
}

fn parse_search(data: &Value, kind: SearchKind) -> Vec<Item> {
    find_all(data, "musicResponsiveListItemRenderer")
        .into_iter()
        .filter_map(|r| list_item(r, None))
        .filter(|item| match kind {
            SearchKind::Songs => item.kind == ItemKind::Song,
            SearchKind::Albums => item.kind == ItemKind::Album,
            SearchKind::Artists => item.kind == ItemKind::Artist,
            SearchKind::Playlists => item.kind == ItemKind::Playlist,
        })
        .collect()
}

/// Songs in list rows (album tracks, top songs, playlist items).
fn songs_in(data: &Value, album_artist: Option<&str>) -> Vec<Track> {
    find_all(data, "musicResponsiveListItemRenderer")
        .into_iter()
        .filter_map(|r| list_item(r, album_artist))
        .filter_map(|item| item.track)
        .collect()
}

/// Carousels of cards (and of song rows) with their titles.
fn parse_shelves(data: &Value) -> Vec<Shelf> {
    find_all(data, "musicCarouselShelfRenderer")
        .into_iter()
        .filter_map(|shelf| {
            let title = text(shelf.pointer("/header/musicCarouselShelfBasicHeaderRenderer/title"));
            let items: Vec<Item> = shelf
                .get("contents")?
                .as_array()?
                .iter()
                .filter_map(|c| {
                    c.get("musicTwoRowItemRenderer").and_then(card).or_else(|| {
                        c.get("musicResponsiveListItemRenderer")
                            .and_then(|r| list_item(r, None))
                    })
                })
                .collect();
            (!items.is_empty()).then_some(Shelf { title, items })
        })
        .collect()
}

/// The playlist behind an artist page's "Top songs" shelf.
fn top_songs_playlist(data: &Value) -> Option<String> {
    find_all(data, "musicShelfRenderer")
        .into_iter()
        .find_map(|shelf| {
            shelf
                .get("bottomEndpoint")
                .or_else(|| shelf.pointer("/title/runs/0/navigationEndpoint"))
                .and_then(|e| e.pointer("/browseEndpoint/browseId"))
                .and_then(Value::as_str)
                .filter(|id| id.starts_with("VL"))
                .map(str::to_owned)
        })
}

fn parse_page(id: &str, data: &Value) -> Page {
    let kind = if id.starts_with("MPRE") {
        ItemKind::Album
    } else if id.starts_with("UC") {
        ItemKind::Artist
    } else {
        ItemKind::Playlist
    };
    let header = [
        "musicImmersiveHeaderRenderer",
        "musicVisualHeaderRenderer",
        "musicResponsiveHeaderRenderer",
        "musicDetailHeaderRenderer",
    ]
    .iter()
    .find_map(|key| find_first(data, key));
    let title = header.map(|h| text(h.get("title"))).unwrap_or_default();
    let strapline = header
        .map(|h| text(h.get("straplineTextOne")))
        .unwrap_or_default();
    let subtitle = match (header.map(|h| text(h.get("subtitle"))), kind) {
        (Some(s), ItemKind::Album) if !strapline.is_empty() => format!("{s} • {strapline}"),
        (Some(s), _) if !s.is_empty() => s,
        _ => header
            .and_then(|h| h.get("monthlyListenerCount"))
            .map(|c| text(Some(c)))
            .unwrap_or_default(),
    };
    let thumbnail = header
        .and_then(|h| h.get("thumbnail"))
        .and_then(thumbnail)
        .map(|t| t.to_string());
    let channel_id = header
        .and_then(|h| find_first(h, "subscribeButtonRenderer"))
        .and_then(|s| s.get("channelId"))
        .and_then(Value::as_str)
        .map(str::to_owned);

    // Track lists live in shelves (not carousels).
    let album_artist =
        (kind == ItemKind::Album && !strapline.is_empty()).then_some(strapline.as_str());
    let tracks: Vec<Track> = ["musicShelfRenderer", "musicPlaylistShelfRenderer"]
        .iter()
        .flat_map(|key| find_all(data, key))
        .flat_map(|shelf| songs_in(shelf, album_artist))
        .collect();
    let shelves = parse_shelves(data)
        .into_iter()
        // Video carousels duplicate the songs; keep albums, artists, lists.
        .filter(|s| s.items.iter().any(|i| i.kind != ItemKind::Song))
        .collect();
    Page {
        kind,
        id: id.to_owned(),
        title,
        subtitle,
        thumbnail,
        tracks,
        shelves,
        channel_id,
    }
}

fn parse_radio(data: &Value) -> Vec<Track> {
    find_all(data, "playlistPanelVideoRenderer")
        .into_iter()
        .filter_map(|r| {
            let video_id = r.get("videoId")?.as_str()?;
            let title = text(r.get("title"));
            let artist = runs(r.get("longBylineText"))
                .first()
                .and_then(|run| run.get("text"))
                .and_then(Value::as_str)
                .unwrap_or_default();
            Some(Track {
                video_id: video_id.into(),
                title: title.as_str().into(),
                artist: clean_artist(artist),
                duration_secs: parse_duration(&text(r.get("lengthText"))),
            })
        })
        .collect()
}

fn tab_browse_id(next: &Value, title: &str) -> Option<String> {
    find_all(next, "tabRenderer")
        .into_iter()
        .find(|tab| tab.get("title").and_then(Value::as_str) == Some(title))?
        .pointer("/endpoint/browseEndpoint/browseId")?
        .as_str()
        .map(str::to_owned)
}

fn parse_lyrics(data: &Value) -> Option<Lyrics> {
    let shelf = find_first(data, "musicDescriptionShelfRenderer")?;
    let text_ = text(shelf.get("description"));
    (!text_.is_empty()).then(|| Lyrics {
        text: text_,
        source: text(shelf.get("footer")),
    })
}

fn parse_loudness(data: &Value) -> Option<f32> {
    let per_format = data
        .pointer("/streamingData/adaptiveFormats")
        .and_then(Value::as_array)
        .and_then(|formats| {
            formats
                .iter()
                .find(|f| f.get("itag").and_then(Value::as_u64) == Some(140))
        })
        .and_then(|f| f.get("loudnessDb"))
        .and_then(Value::as_f64);
    let track = || {
        let config = data.pointer("/playerConfig/audioConfig")?;
        let absolute = config.get("trackAbsoluteLoudnessLkfs")?.as_f64()?;
        let target = config.get("loudnessTargetLkfs")?.as_f64()?;
        Some(absolute - target)
    };
    per_format.or_else(track).map(|db| db as f32)
}

fn continuation(data: &Value) -> Option<String> {
    find_first(data, "nextContinuationData")
        .and_then(|c| c.get("continuation"))
        .or_else(|| find_first(data, "continuationCommand").and_then(|c| c.get("token")))
        .and_then(Value::as_str)
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn browse(id: &str, page_type: &str) -> Value {
        json!({ "browseEndpoint": { "browseId": id,
            "browseEndpointContextSupportedConfigs": { "browseEndpointContextMusicConfig": { "pageType": page_type } } } })
    }

    fn column(runs: Value) -> Value {
        json!({ "musicResponsiveListItemFlexColumnRenderer": { "text": { "runs": runs } } })
    }

    fn song_row() -> Value {
        json!({ "musicResponsiveListItemRenderer": {
            "flexColumns": [
                column(json!([{ "text": "Dancing Queen" }])),
                column(json!([
                    { "text": "ABBA", "navigationEndpoint": browse("UCabba", "MUSIC_PAGE_TYPE_ARTIST") },
                    { "text": " • " },
                    { "text": "Arrival", "navigationEndpoint": browse("MPREb_x", "MUSIC_PAGE_TYPE_ALBUM") },
                    { "text": " • " },
                    { "text": "3:52" }
                ]))
            ],
            "playlistItemData": { "videoId": "YkLLcIKhJ64" },
            "thumbnail": { "musicThumbnailRenderer": { "thumbnail": { "thumbnails": [
                { "url": "small", "width": 60 }, { "url": "big", "width": 226 } ] } } }
        } })
    }

    fn album_row() -> Value {
        json!({ "musicResponsiveListItemRenderer": {
            "flexColumns": [
                column(json!([{ "text": "ABBA Gold" }])),
                column(json!([{ "text": "Album" }, { "text": " • " }, { "text": "ABBA" }, { "text": " • " }, { "text": "1992" }]))
            ],
            "navigationEndpoint": browse("MPREb_gold", "MUSIC_PAGE_TYPE_ALBUM")
        } })
    }

    #[test]
    fn search_rows() {
        let data = json!({ "contents": [ song_row(), album_row() ] });
        let songs = parse_search(&data, SearchKind::Songs);
        assert_eq!(songs.len(), 1);
        let t = songs[0].track.as_ref().unwrap();
        assert_eq!(
            (&*t.title, &*t.artist, t.duration_secs),
            ("Dancing Queen", "ABBA", Some(232))
        );
        assert_eq!(songs[0].thumbnail.as_deref(), Some("big"));

        let albums = parse_search(&data, SearchKind::Albums);
        assert_eq!(albums.len(), 1);
        assert_eq!(
            (albums[0].kind, &*albums[0].id),
            (ItemKind::Album, "MPREb_gold")
        );
        assert_eq!(&*albums[0].subtitle, "Album • ABBA • 1992");
    }

    #[test]
    fn album_page_uses_the_album_artist() {
        let row = json!({ "musicResponsiveListItemRenderer": {
            "flexColumns": [ column(json!([{ "text": "Mamma Mia" }])), column(json!([])) ],
            "fixedColumns": [ { "musicResponsiveListItemFixedColumnRenderer": { "text": { "runs": [ { "text": "3:33" } ] } } } ],
            "playlistItemData": { "videoId": "unFR0ZXmB0E" }
        } });
        let data = json!({
            "header": { "musicResponsiveHeaderRenderer": {
                "title": { "runs": [ { "text": "ABBA Gold" } ] },
                "subtitle": { "runs": [ { "text": "Album" }, { "text": " • " }, { "text": "1992" } ] },
                "straplineTextOne": { "runs": [ { "text": "ABBA" } ] }
            } },
            "contents": { "musicShelfRenderer": { "contents": [ row ] } }
        });
        let page = parse_page("MPREb_gold", &data);
        assert_eq!(page.kind, ItemKind::Album);
        assert_eq!(page.title, "ABBA Gold");
        assert_eq!(page.subtitle, "Album • 1992 • ABBA");
        assert_eq!(page.tracks.len(), 1);
        assert_eq!(&*page.tracks[0].artist, "ABBA");
        assert_eq!(page.tracks[0].duration_secs, Some(213));
    }

    #[test]
    fn artist_page_sections() {
        let data = json!({
            "header": { "musicImmersiveHeaderRenderer": {
                "title": { "runs": [ { "text": "ABBA" } ] },
                "subscriptionButton": { "subscribeButtonRenderer": { "channelId": "UCyt" } }
            } },
            "contents": [
                { "musicShelfRenderer": { "contents": [ song_row() ] } },
                { "musicCarouselShelfRenderer": {
                    "header": { "musicCarouselShelfBasicHeaderRenderer": { "title": { "runs": [ { "text": "Albums" } ] } } },
                    "contents": [ { "musicTwoRowItemRenderer": {
                        "title": { "runs": [ { "text": "Arrival" } ] },
                        "subtitle": { "runs": [ { "text": "1976" } ] },
                        "navigationEndpoint": browse("MPREb_arrival", "MUSIC_PAGE_TYPE_ALBUM")
                    } } ]
                } },
                { "musicCarouselShelfRenderer": {
                    "header": { "musicCarouselShelfBasicHeaderRenderer": { "title": { "runs": [ { "text": "Videos" } ] } } },
                    "contents": [ { "musicTwoRowItemRenderer": {
                        "title": { "runs": [ { "text": "Dancing Queen" } ] },
                        "navigationEndpoint": { "watchEndpoint": { "videoId": "x" } }
                    } } ]
                } }
            ]
        });
        let page = parse_page("UCabba", &data);
        assert_eq!(page.kind, ItemKind::Artist);
        assert_eq!(page.channel_id.as_deref(), Some("UCyt"));
        assert_eq!(page.tracks.len(), 1, "top songs");
        assert_eq!(page.shelves.len(), 1, "video carousel dropped");
        assert_eq!(page.shelves[0].title, "Albums");
        assert_eq!(page.shelves[0].items[0].kind, ItemKind::Album);
    }

    #[test]
    fn radio_and_tabs() {
        let data = json!({
            "contents": [
                { "playlistPanelVideoRenderer": {
                    "videoId": "5mHzaIehRTE",
                    "title": { "runs": [ { "text": "Lay All Your Love On Me" } ] },
                    "longBylineText": { "runs": [ { "text": "ABBA" }, { "text": " • " }, { "text": "Super Trouper" } ] },
                    "lengthText": { "runs": [ { "text": "4:35" } ] }
                } },
                { "tabRenderer": { "title": "Lyrics", "endpoint": { "browseEndpoint": { "browseId": "MPLYt_x" } } } }
            ]
        });
        let radio = parse_radio(&data);
        assert_eq!(radio.len(), 1);
        assert_eq!(
            (&*radio[0].artist, radio[0].duration_secs),
            ("ABBA", Some(275))
        );
        assert_eq!(tab_browse_id(&data, "Lyrics").as_deref(), Some("MPLYt_x"));
        assert_eq!(tab_browse_id(&data, "Related"), None);
    }

    #[test]
    fn loudness_from_the_aac_format_else_the_track() {
        let data = json!({ "streamingData": { "adaptiveFormats": [
            { "itag": 251, "loudnessDb": 1.0 }, { "itag": 140, "loudnessDb": 3.68 } ] } });
        assert_eq!(parse_loudness(&data), Some(3.68));
        let data = json!({ "playerConfig": { "audioConfig": {
            "trackAbsoluteLoudnessLkfs": -10.0, "loudnessTargetLkfs": -14.0 } } });
        assert_eq!(parse_loudness(&data), Some(4.0));
        assert_eq!(parse_loudness(&json!({})), None);
    }

    #[test]
    fn lyrics_and_durations() {
        let data = json!({ "musicDescriptionShelfRenderer": {
            "description": { "runs": [ { "text": "Half-past twelve\nAnd I'm watching" } ] },
            "footer": { "runs": [ { "text": "Source: Musixmatch" } ] }
        } });
        let lyrics = parse_lyrics(&data).unwrap();
        assert!(lyrics.text.starts_with("Half-past"));
        assert_eq!(lyrics.source, "Source: Musixmatch");
        assert_eq!(parse_duration("1:02:03"), Some(3723));
        assert_eq!(parse_duration("1.7B plays"), None);
        assert_eq!(parse_duration("42"), None);
    }

    /// Network: the real endpoints still parse.
    #[tokio::test]
    #[ignore]
    async fn live_catalog() {
        let catalog = Catalog::new(reqwest::Client::new());
        let songs = catalog
            .search("abba dancing queen", SearchKind::Songs)
            .await
            .unwrap();
        assert!(songs.iter().any(|s| s.track.is_some()), "{songs:?}");
        let albums = catalog.search("abba", SearchKind::Albums).await.unwrap();
        let album = catalog.page(&albums[0].id).await.unwrap();
        assert!(!album.tracks.is_empty(), "{album:?}");
        let artists = catalog.search("abba", SearchKind::Artists).await.unwrap();
        let artist = catalog.page(&artists[0].id).await.unwrap();
        assert!(
            artist.tracks.len() > 5 && !artist.shelves.is_empty(),
            "the full top songs: {artist:?}"
        );
        assert!(artist.tracks.iter().all(|t| t.duration_secs.is_some()));
        assert_eq!(
            catalog.artist_id("uelHwf8o7_U").await.unwrap().as_deref(),
            Some("UCedvOgsKFzcK3hA5taf3KoQ"),
            "Eminem, not Rihanna"
        );
        let radio = catalog.radio("YkLLcIKhJ64").await.unwrap();
        assert!(radio.len() > 10);
        assert!(catalog.lyrics("YkLLcIKhJ64").await.unwrap().is_some());
        assert!(!catalog.home().await.unwrap().is_empty());
    }

    /// Separate from `live_catalog`: the player endpoint challenges
    /// datacenter IPs (CI), unlike the browse / search ones.
    #[tokio::test]
    #[ignore]
    async fn live_loudness() {
        let catalog = Catalog::new(reqwest::Client::new());
        assert!(catalog.loudness_db("YkLLcIKhJ64").await.unwrap().is_some());
    }
}

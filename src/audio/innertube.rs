//! Media URLs straight from YouTube's player endpoint, without yt-dlp: the
//! phones' resolver (YTMKit `Resolver.swift`, Kotlin `Resolver.kt`) for
//! the desktop, anonymous clients only.
//!
//! The player endpoint is asked as an InnerTube client that still gets
//! plain (unciphered, no PO token) AAC URLs — VISIONOS since 2026-10-12 —
//! with a `visitorData` from the small `visitor_id` endpoint (without one,
//! YouTube asks "confirm you're not a bot" after a few requests). Measured
//! 2026-11-27 from a Linux desktop: ~0.13 s per URL (yt-dlp: 2–6 s and
//! ~90 MB), itag 140, every range 206 (after googlevideo's 302).
//!
//! Signed-in, age-restricted and ciphered cases are not here: those
//! [`DirectError::Refused`] answers make the resolver ask yt-dlp. The list
//! of clients can be replaced without a release: `ios/clients.json` on
//! main, shared with the phones (only its anonymous plain entries apply).

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, RwLock},
    time::{Duration, Instant, SystemTime},
};

use futures_util::future::BoxFuture;
use serde::Deserialize;
use serde_json::{Value, json};

use super::media_url::{AudioStream, Via, expiry_from_url};

const PLAYER: &str = "https://www.youtube.com/youtubei/v1/player?prettyPrint=false";
const VISITOR: &str = "https://www.youtube.com/youtubei/v1/visitor_id?prettyPrint=false";
const REMOTE_PROFILES: &str =
    "https://raw.githubusercontent.com/yaroven/YTMusicPlayer/main/ios/clients.json";
/// Per request: a slow answer is a network problem, not a reason for yt-dlp.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// How long one `visitorData` is used (the phones keep theirs an hour).
const VISITOR_TTL: Duration = Duration::from_secs(60 * 60);
/// AAC-LC 128 kbps in MP4: what rodio's symphonia decodes.
const AAC_ITAG: u64 = 140;

/// What turns a video id into a media URL without yt-dlp: [`InnerTube`] in
/// the app, scripted fakes in tests.
pub trait DirectResolver: Send + Sync + 'static {
    fn resolve<'a>(&'a self, video_id: &'a str) -> BoxFuture<'a, Result<AudioStream, DirectError>>;

    /// Background upkeep at startup (the client list).
    fn maintain(&self) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }
}

/// Why no URL came, and so whether yt-dlp is worth starting.
#[derive(Debug, thiserror::Error)]
pub enum DirectError {
    /// The request didn't get through or its answer didn't come: yt-dlp
    /// talks to the same servers, so it isn't tried (the session waits for
    /// the connection instead).
    #[error("YouTube didn't answer")]
    Network(#[source] reqwest::Error),
    /// YouTube answered without a plain AAC URL (bot check, sign-in or age
    /// check, no such format, a changed client): yt-dlp may still get one.
    #[error("{0}")]
    Refused(String),
}

impl From<reqwest::Error> for DirectError {
    fn from(err: reqwest::Error) -> Self {
        if err.is_connect() || err.is_timeout() {
            Self::Network(err)
        } else {
            Self::Refused(err.to_string())
        }
    }
}

/// An InnerTube client the player endpoint is asked as (`ios/clients.json`).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Profile {
    name: String,
    version: String,
    /// `X-YouTube-Client-Name`.
    id: String,
    user_agent: String,
    device_make: Option<String>,
    device_model: Option<String>,
    os_name: Option<String>,
    os_version: Option<String>,
    /// Signed-in / ciphered / HLS clients are the phones' (yt-dlp here).
    #[serde(default)]
    auth: bool,
    #[serde(default)]
    cipher: bool,
    #[serde(default)]
    hls: bool,
}

impl Profile {
    /// Measured 2026-10-12 (phones) and 2026-11-27 (Linux desktop).
    fn vision_os() -> Self {
        Self {
            name: "VISIONOS".into(),
            version: "1.02".into(),
            id: "101".into(),
            user_agent: "Mozilla/5.0 (Macintosh; Intel Mac OS X 15_7_3) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.0 Safari/605.1.15".into(),
            device_make: Some("Apple".into()),
            device_model: Some("RealityDevice17,1".into()),
            os_name: Some("visionOS".into()),
            os_version: Some("26.5.23O471".into()),
            auth: false,
            cipher: false,
            hls: false,
        }
    }

    /// Hands out plain URLs without an account: usable here.
    fn is_plain(&self) -> bool {
        !self.auth && !self.cipher && !self.hls
    }

    fn context(&self, visitor: Option<&str>) -> Value {
        let mut client = json!({
            "clientName": self.name,
            "clientVersion": self.version,
            "userAgent": self.user_agent,
            "hl": "en",
            "gl": "US",
        });
        for (key, value) in [
            ("deviceMake", &self.device_make),
            ("deviceModel", &self.device_model),
            ("osName", &self.os_name),
            ("osVersion", &self.os_version),
        ] {
            if let Some(value) = value {
                client[key] = json!(value);
            }
        }
        if let Some(visitor) = visitor {
            client["visitorData"] = json!(visitor);
        }
        json!({ "client": client })
    }
}

/// The player endpoint as anonymous InnerTube clients.
pub struct InnerTube {
    http: reqwest::Client,
    profiles: RwLock<Arc<[Profile]>>,
    visitor: Mutex<Option<(String, Instant)>>,
}

impl InnerTube {
    pub fn new(http: reqwest::Client) -> Self {
        Self {
            http,
            profiles: RwLock::new(Arc::from([Profile::vision_os()])),
            visitor: Mutex::new(None),
        }
    }

    async fn resolve_inner(&self, video_id: &str) -> Result<AudioStream, DirectError> {
        let profiles = self
            .profiles
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let mut refused = String::from("no InnerTube client");
        for profile in profiles.iter() {
            match self.ask(video_id, profile).await {
                Ok(stream) => return Ok(stream),
                Err(DirectError::Refused(why)) if is_bot_check(&why) => {
                    // A stale or missing visitor id: once more with a new one.
                    self.forget_visitor();
                    match self.ask(video_id, profile).await {
                        Ok(stream) => return Ok(stream),
                        Err(DirectError::Refused(why)) => refused = why,
                        Err(err) => return Err(err),
                    }
                }
                Err(DirectError::Refused(why)) => refused = why,
                Err(err) => return Err(err),
            }
        }
        Err(DirectError::Refused(refused))
    }

    /// One player request as `profile`.
    async fn ask(&self, video_id: &str, profile: &Profile) -> Result<AudioStream, DirectError> {
        let visitor = self.visitor().await?;
        let body = json!({
            "videoId": video_id,
            "contentCheckOk": true,
            "racyCheckOk": true,
            "context": profile.context(visitor.as_deref()),
        });
        let mut request = self
            .http
            .post(PLAYER)
            .timeout(REQUEST_TIMEOUT)
            .header("User-Agent", &profile.user_agent)
            .header("X-YouTube-Client-Name", &profile.id)
            .header("X-YouTube-Client-Version", &profile.version)
            .header("Origin", "https://www.youtube.com")
            .json(&body);
        if let Some(visitor) = &visitor {
            request = request.header("X-Goog-Visitor-Id", visitor);
        }
        let response = request.send().await?;
        let status = response.status();
        if !status.is_success() {
            return Err(DirectError::Refused(format!(
                "{} player: HTTP {status}",
                profile.name
            )));
        }
        let data: Value = response.json().await?;
        parse_player(video_id, &data, &profile.user_agent, SystemTime::now())
            .map_err(DirectError::Refused)
    }

    /// A visitor id, fetched once an hour (`None` when YouTube gave none:
    /// the player is asked without one).
    async fn visitor(&self) -> Result<Option<String>, DirectError> {
        if let Some((value, at)) = &*self.visitor.lock().unwrap_or_else(|e| e.into_inner())
            && at.elapsed() < VISITOR_TTL
        {
            return Ok(Some(value.clone()));
        }
        let body = json!({
            "context": { "client": {
                "clientName": "WEB", "clientVersion": "2.20260708.00.00", "hl": "en", "gl": "US",
            }}
        });
        let response = self
            .http
            .post(VISITOR)
            .timeout(REQUEST_TIMEOUT)
            .json(&body)
            .send()
            .await?;
        let data: Value = match response.error_for_status() {
            Ok(response) => response.json().await?,
            Err(err) => {
                tracing::debug!(%err, "visitor id");
                return Ok(None);
            }
        };
        let value = data
            .pointer("/responseContext/visitorData")
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty())
            .map(str::to_owned);
        if let Some(value) = &value {
            *self.visitor.lock().unwrap_or_else(|e| e.into_inner()) =
                Some((value.clone(), Instant::now()));
        }
        Ok(value)
    }

    fn forget_visitor(&self) {
        *self.visitor.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    /// The client list from `ios/clients.json` on main, when it has
    /// anonymous plain clients; the built-in one otherwise.
    async fn refresh_profiles(&self) {
        let fetched = async {
            let text = self
                .http
                .get(REMOTE_PROFILES)
                .timeout(Duration::from_secs(5))
                .header("Cache-Control", "no-cache")
                .send()
                .await?
                .error_for_status()?
                .text()
                .await?;
            anyhow::Ok(serde_json::from_str::<Vec<Profile>>(&text)?)
        }
        .await;
        match fetched {
            Ok(list) => {
                let plain: Vec<Profile> = list.into_iter().filter(Profile::is_plain).collect();
                if plain.is_empty() {
                    return;
                }
                tracing::debug!(clients = ?plain.iter().map(|p| &p.name).collect::<Vec<_>>(), "InnerTube clients");
                *self.profiles.write().unwrap_or_else(|e| e.into_inner()) = Arc::from(plain);
            }
            Err(err) => tracing::debug!(%err, "InnerTube client list"),
        }
    }
}

impl DirectResolver for InnerTube {
    fn resolve<'a>(&'a self, video_id: &'a str) -> BoxFuture<'a, Result<AudioStream, DirectError>> {
        Box::pin(self.resolve_inner(video_id))
    }

    fn maintain(&self) -> BoxFuture<'_, ()> {
        Box::pin(self.refresh_profiles())
    }
}

/// "Sign in to confirm you're not a bot": the visitor id, not the song.
fn is_bot_check(reason: &str) -> bool {
    reason.to_lowercase().contains("not a bot")
}

fn mime(format: &Value) -> &str {
    format
        .get("mimeType")
        .and_then(Value::as_str)
        .unwrap_or_default()
}

/// The AAC (itag 140) stream of a player response, or why there is none.
fn parse_player(
    video_id: &str,
    data: &Value,
    user_agent: &str,
    now: SystemTime,
) -> Result<AudioStream, String> {
    let text = |path: &str| data.pointer(path).and_then(Value::as_str);
    let status = text("/playabilityStatus/status").unwrap_or_default();
    if status != "OK" {
        let reason = text("/playabilityStatus/reason")
            .filter(|r| !r.is_empty())
            .unwrap_or(if status.is_empty() {
                "no answer"
            } else {
                status
            });
        return Err(format!("{status}: {reason}"));
    }
    let formats: Vec<&Value> = ["/streamingData/adaptiveFormats", "/streamingData/formats"]
        .iter()
        .filter_map(|p| data.pointer(p).and_then(Value::as_array))
        .flatten()
        .filter(|f| f.get("url").and_then(Value::as_str).is_some())
        .collect();
    let format = formats
        .iter()
        .find(|f| f.get("itag").and_then(Value::as_u64) == Some(AAC_ITAG))
        .or_else(|| formats.iter().find(|f| mime(f).starts_with("audio/mp4")))
        .ok_or("no plain AAC format")?;
    let url = format["url"].as_str().unwrap_or_default().to_owned();
    let number = |f: &Value, key: &str| -> Option<f64> {
        match f.get(key)? {
            Value::String(s) => s.parse().ok(),
            v => v.as_f64(),
        }
    };
    let expires_at = expiry_from_url(&url).or_else(|| {
        number(data.pointer("/streamingData")?, "expiresInSeconds")
            .map(|secs| now + Duration::from_secs_f64(secs.max(0.0)))
    });
    let duration = number(format, "approxDurationMs")
        .map(|ms| ms / 1000.0)
        .or_else(|| number(data.pointer("/videoDetails")?, "lengthSeconds"))
        .filter(|s| s.is_finite() && *s > 0.0)
        .map(Duration::from_secs_f64);
    let codec = mime(format)
        .split_once("codecs=\"")
        .and_then(|(_, rest)| rest.split('"').next())
        .map(str::to_owned);
    Ok(AudioStream {
        video_id: video_id.to_owned(),
        title: text("/videoDetails/title").map(str::to_owned),
        url,
        format_id: format
            .get("itag")
            .and_then(Value::as_u64)
            .map(|i| i.to_string()),
        ext: Some(
            if mime(format).starts_with("audio/mp4") {
                "m4a"
            } else {
                "mp4"
            }
            .into(),
        ),
        codec,
        bitrate_kbps: number(format, "averageBitrate")
            .or_else(|| number(format, "bitrate"))
            .map(|b| b / 1000.0),
        sample_rate: number(format, "audioSampleRate").map(|r| r as u32),
        duration,
        content_length: number(format, "contentLength").map(|n| n as u64),
        http_headers: HashMap::from([("User-Agent".to_owned(), user_agent.to_owned())]),
        expires_at,
        hls: false,
        via: Via::InnerTube,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn answer(status: &str, reason: Option<&str>, formats: Value) -> Value {
        json!({
            "playabilityStatus": { "status": status, "reason": reason },
            "videoDetails": { "title": "Never Gonna Give You Up", "lengthSeconds": "213" },
            "streamingData": { "expiresInSeconds": "21540", "adaptiveFormats": formats },
        })
    }

    fn aac(url: &str) -> Value {
        json!({
            "itag": 140, "url": url,
            "mimeType": "audio/mp4; codecs=\"mp4a.40.2\"",
            "averageBitrate": 129_478, "audioSampleRate": "44100",
            "contentLength": "3449447", "approxDurationMs": "213089",
        })
    }

    #[test]
    fn takes_the_aac_format() {
        let url = "https://rr1.googlevideo.com/videoplayback?expire=1900000000&itag=140";
        let formats = json!([
            { "itag": 251, "url": "https://x/opus", "mimeType": "audio/webm; codecs=\"opus\"" },
            aac(url),
        ]);
        let now = SystemTime::now();
        let s = parse_player("dQw4w9WgXcQ", &answer("OK", None, formats), "UA", now).unwrap();
        assert_eq!(s.url, url);
        assert_eq!(s.format_id.as_deref(), Some("140"));
        assert_eq!(s.ext.as_deref(), Some("m4a"));
        assert_eq!(s.codec.as_deref(), Some("mp4a.40.2"));
        assert_eq!(s.content_length, Some(3_449_447));
        assert_eq!(s.sample_rate, Some(44_100));
        assert_eq!(s.duration, Some(Duration::from_millis(213_089)));
        assert_eq!(s.title.as_deref(), Some("Never Gonna Give You Up"));
        assert_eq!(
            s.http_headers.get("User-Agent").map(String::as_str),
            Some("UA")
        );
        assert_eq!(
            s.expires_at,
            Some(std::time::UNIX_EPOCH + Duration::from_secs(1_900_000_000))
        );
        assert_eq!(s.via, Via::InnerTube);
        assert!(s.is_decodable() && s.is_plain_file());
    }

    #[test]
    fn says_why_when_there_is_nothing_to_play() {
        let bot = answer(
            "LOGIN_REQUIRED",
            Some("Sign in to confirm you’re not a bot"),
            json!([]),
        );
        let why = parse_player("x", &bot, "UA", SystemTime::now()).unwrap_err();
        assert!(is_bot_check(&why), "{why}");

        let age = answer(
            "LOGIN_REQUIRED",
            Some("Sign in to confirm your age"),
            json!([]),
        );
        let why = parse_player("x", &age, "UA", SystemTime::now()).unwrap_err();
        assert!(!is_bot_check(&why), "{why}");

        // Ciphered formats (no `url`) are not plain URLs.
        let ciphered = answer(
            "OK",
            None,
            json!([{ "itag": 140, "signatureCipher": "s=..&url=..", "mimeType": "audio/mp4" }]),
        );
        assert!(parse_player("x", &ciphered, "UA", SystemTime::now()).is_err());
    }

    #[test]
    fn only_plain_clients_from_the_shared_list() {
        let list: Vec<Profile> =
            serde_json::from_str(include_str!("../../ios/clients.json")).unwrap();
        let plain: Vec<&str> = list
            .iter()
            .filter(|p| p.is_plain())
            .map(|p| p.name.as_str())
            .collect();
        assert_eq!(plain, ["VISIONOS"]);
    }

    /// The real endpoint: `cargo test -- --ignored innertube`.
    #[tokio::test]
    #[ignore = "network"]
    async fn live_innertube_url_plays_in_ranges() {
        let http = reqwest::Client::new();
        let resolver = InnerTube::new(http.clone());
        let stream = resolver.resolve("dQw4w9WgXcQ").await.unwrap();
        let len = stream.content_length.unwrap();
        for start in [0, len / 2, len - 1024] {
            let response = http
                .get(&stream.url)
                .header("Range", format!("bytes={start}-{}", start + 1023))
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), 206, "range at {start}");
        }
    }
}

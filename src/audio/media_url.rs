//! A resolved media URL: what every URL source (InnerTube, yt-dlp) hands
//! the rest of the pipeline, whichever of them found it.

use std::{
    collections::HashMap,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

/// Who found a URL. A URL that stops working tells the resolver which
/// source to distrust.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Via {
    /// YouTube's player endpoint, asked directly (`audio::innertube`).
    InnerTube,
    /// A yt-dlp run (`audio::extractor`). URLs cached before InnerTube
    /// existed have no `via`: they came from yt-dlp.
    #[default]
    YtDlp,
}

/// A direct, time-limited audio URL plus what's needed to play it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioStream {
    pub video_id: String,
    pub title: Option<String>,
    pub url: String,
    pub format_id: Option<String>,
    /// Container extension, e.g. `m4a` or `webm`.
    pub ext: Option<String>,
    /// Codec, e.g. `mp4a.40.2` or `opus`.
    pub codec: Option<String>,
    pub bitrate_kbps: Option<f64>,
    pub sample_rate: Option<u32>,
    pub duration: Option<Duration>,
    pub content_length: Option<u64>,
    /// Headers the media server expects (User-Agent etc.).
    pub http_headers: HashMap<String, String>,
    /// From the URL's `expire` param; googlevideo URLs live ~6h.
    pub expires_at: Option<SystemTime>,
    /// `url` is an HLS media playlist (age-restricted songs, signed in:
    /// see `audio::hls`).
    #[serde(default)]
    pub hls: bool,
    #[serde(default)]
    pub via: Via,
}

impl AudioStream {
    /// True when the URL is still valid for at least `margin` from now.
    pub fn is_fresh(&self, margin: Duration) -> bool {
        self.expires_at
            .is_some_and(|exp| SystemTime::now() + margin < exp)
    }

    /// True for codecs our rodio/symphonia build can decode.
    pub fn is_decodable(&self) -> bool {
        matches!(
            self.ext.as_deref(),
            Some("m4a" | "mp4" | "mp3" | "ogg" | "flac" | "wav")
        ) && !self.codec.as_deref().is_some_and(|c| c.starts_with("opus"))
    }

    /// A plain file over HTTP of known length that we can fetch ourselves
    /// in ranges (downloads), not an HLS playlist.
    pub fn is_plain_file(&self) -> bool {
        !self.hls && self.ext.as_deref() == Some("m4a") && self.content_length.is_some()
    }
}

/// The `expire` query parameter of a googlevideo URL.
pub fn expiry_from_url(raw: &str) -> Option<SystemTime> {
    let url = url::Url::parse(raw).ok()?;
    let secs = url
        .query_pairs()
        .find(|(k, _)| k == "expire")
        .and_then(|(_, v)| v.parse::<u64>().ok())?;
    Some(UNIX_EPOCH + Duration::from_secs(secs))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_cached_before_innertube_came_from_ytdlp() {
        let old = r#"{"video_id":"dQw4w9WgXcQ","title":null,"url":"https://x","format_id":"140",
            "ext":"m4a","codec":"mp4a.40.2","bitrate_kbps":null,"sample_rate":null,"duration":null,
            "content_length":null,"http_headers":{},"expires_at":null}"#;
        let stream: AudioStream = serde_json::from_str(old).unwrap();
        assert_eq!(stream.via, Via::YtDlp);
        assert!(!stream.hls);
    }

    #[test]
    fn expiry_comes_from_the_url() {
        let at = expiry_from_url("https://r1.googlevideo.com/videoplayback?expire=1700000000&x=1");
        assert_eq!(at, Some(UNIX_EPOCH + Duration::from_secs(1_700_000_000)));
        assert_eq!(expiry_from_url("not a url"), None);
    }
}

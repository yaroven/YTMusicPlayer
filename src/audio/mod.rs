//! Audio pipeline: yt-dlp URL resolution, HTTP streaming, rodio playback.

pub mod extractor;
pub mod install;
pub mod js_runtime;
pub mod player;
pub mod queue;
pub mod resolver;
pub mod stream;

use anyhow::{Result, bail};

use self::{extractor::AudioStream, resolver::StreamResolver, stream::HttpStream};

/// Resolves `video_id` and starts downloading it. A 403/410 on the media URL
/// drops the cached URL and retries once (switching to JS resolution if
/// allowed by the resolver's policy).
pub async fn open_track(
    resolver: &StreamResolver,
    http: &reqwest::Client,
    video_id: &str,
) -> Result<(HttpStream, AudioStream)> {
    let stream = resolver.resolve(video_id).await?;
    if !stream.is_decodable() {
        bail!(
            "unsupported audio format {:?}/{:?}",
            stream.ext.as_deref().unwrap_or("?"),
            stream.codec.as_deref().unwrap_or("?")
        );
    }
    match HttpStream::open(http, &stream, Some(refresher(resolver, video_id))).await {
        Ok(body) => Ok((body, stream)),
        Err(err) if err.is_forbidden() => {
            tracing::info!(video_id, %err, "media URL rejected, re-resolving");
            resolver.report_playback_failure(video_id);
            let stream = resolver.resolve(video_id).await?;
            let body = HttpStream::open(http, &stream, Some(refresher(resolver, video_id))).await?;
            Ok((body, stream))
        }
        Err(err) => Err(err.into()),
    }
}

/// New URLs for a track whose URL stopped working mid-playback: drops the
/// cached one (and switches to JS-assisted resolution when allowed).
fn refresher(resolver: &StreamResolver, video_id: &str) -> stream::Refresh {
    let (resolver, video_id) = (resolver.clone(), video_id.to_owned());
    Box::new(move || {
        let (resolver, video_id) = (resolver.clone(), video_id.clone());
        Box::pin(async move {
            resolver.report_playback_failure(&video_id);
            match resolver.resolve(&video_id).await {
                Ok(stream) => Some(stream),
                Err(err) => {
                    tracing::warn!(%err, video_id, "re-resolving failed");
                    None
                }
            }
        })
    })
}

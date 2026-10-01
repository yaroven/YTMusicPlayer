use anyhow::{Context, Result, bail};
use std::time::Instant;
use tracing_subscriber::EnvFilter;

use ytm_player::{
    audio::{
        extractor::YtDlp,
        resolver::{JsPolicy, StreamResolver},
    },
    config::paths::AppPaths,
};

#[tokio::main]
async fn main() -> Result<()> {
    let paths = AppPaths::new().context("cannot determine home directory")?;
    let _log_guard = init_logging(&paths);

    let args: Vec<String> = std::env::args().skip(1).collect();
    match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        // Temporary dev command until the TUI lands: `ytm resolve <video_id>`.
        ["resolve", video_id] => resolve(&paths, video_id, JsPolicy::OnDemand).await,
        ["resolve", video_id, "--js"] => resolve(&paths, video_id, JsPolicy::Always).await,
        [] => bail!("TUI not implemented yet; try `ytm resolve <video_id>`"),
        _ => bail!("usage: ytm [resolve <video_id> [--js]]"),
    }
}

async fn resolve(paths: &AppPaths, video_id: &str, policy: JsPolicy) -> Result<()> {
    let http = reqwest::Client::builder()
        .user_agent(concat!("ytm-player/", env!("CARGO_PKG_VERSION")))
        .build()?;
    let ytdlp = YtDlp::ensure(&http, &paths.bin_dir()).await?;
    println!("yt-dlp: {} ({:?})", ytdlp.path().display(), ytdlp.source());
    let resolver = StreamResolver::new(ytdlp, http, paths.bin_dir(), policy);

    let started = Instant::now();
    let stream = resolver.resolve(video_id).await?;
    println!(
        "resolved in {:.1?} (js: {})",
        started.elapsed(),
        resolver.uses_js()
    );
    let started = Instant::now();
    resolver.resolve(video_id).await?;
    println!("cached in   {:.1?}", started.elapsed());
    println!("title:    {}", stream.title.as_deref().unwrap_or("?"));
    println!(
        "format:   {:?} {:?} {:?}",
        stream.format_id, stream.ext, stream.codec
    );
    println!("bitrate:  {:?} kbps", stream.bitrate_kbps);
    println!("duration: {:?}", stream.duration);
    println!("expires:  {:?}", stream.expires_at);
    println!("playable: {}", stream.is_decodable());
    println!("url:      {}", stream.url);
    Ok(())
}

/// Logs go to a daily-rotated file: stdout/stderr belong to the TUI.
fn init_logging(paths: &AppPaths) -> tracing_appender::non_blocking::WorkerGuard {
    let appender = tracing_appender::rolling::daily(paths.log_dir(), "ytm.log");
    let (writer, guard) = tracing_appender::non_blocking(appender);
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_env("YTM_LOG").unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_writer(writer)
        .with_ansi(false)
        .init();
    guard
}

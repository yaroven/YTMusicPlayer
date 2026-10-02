//! Builds the app's dependencies from settings: the HTTP client, the
//! Library, the Track source (with yt-dlp located or downloaded) and the
//! Account. Shared by the terminal UI, the window and the CLI commands.

use std::{sync::Arc, time::Duration};

use anyhow::{Context, Result};

use crate::{
    account::Account,
    api::token_store::Tokens,
    audio::{JsPolicy, TrackSource, extractor::YtDlp},
    config::{paths::AppPaths, settings::Settings},
    session::Deps,
    storage::Library,
};

/// One client for API, media and art requests. Few hosts and sequential
/// requests: idle sockets aren't kept around.
pub fn http_client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .user_agent(concat!("ytm-player/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(10))
        .pool_max_idle_per_host(1)
        .pool_idle_timeout(Duration::from_secs(30))
        .build()?)
}

/// The managed yt-dlp, downloaded on first use (a system copy if that
/// fails), with the user's extra arguments.
pub async fn ytdlp(paths: &AppPaths, settings: &Settings, http: &reqwest::Client) -> Result<YtDlp> {
    let ytdlp = match YtDlp::find(&paths.bin_dir()) {
        Some(found) => found,
        None => {
            // Visible for CLI commands; the UIs show "loading" meanwhile.
            eprintln!("Downloading yt-dlp (first run)…");
            YtDlp::ensure(http, &paths.bin_dir())
                .await
                .context("yt-dlp is required")?
        }
    };
    Ok(ytdlp.with_extra_args(settings.ytdlp_extra_args.iter().cloned()))
}

/// `force_js` (`ytm resolve --js`) beats the `js_fallback` setting.
pub fn js_policy(settings: &Settings, force_js: bool) -> JsPolicy {
    match (force_js, settings.js_fallback) {
        (true, _) => JsPolicy::Always,
        (false, true) => JsPolicy::OnDemand,
        (false, false) => JsPolicy::Never,
    }
}

pub fn track_source(
    paths: &AppPaths,
    settings: &Settings,
    http: &reqwest::Client,
    ytdlp: YtDlp,
    force_js: bool,
    store: Option<Arc<Library>>,
) -> TrackSource {
    TrackSource::new(
        Arc::new(ytdlp),
        http.clone(),
        paths.bin_dir(),
        js_policy(settings, force_js),
        store,
    )
}

/// Everything a player Session needs; starts yt-dlp's daily update.
pub async fn deps(paths: &AppPaths, settings: &Settings) -> Result<Deps> {
    let http = http_client()?;
    let library = Arc::new(Library::open(&paths.database())?);
    let ytdlp = ytdlp(paths, settings, &http).await?;
    let source = track_source(paths, settings, &http, ytdlp, false, Some(library.clone()));
    source.spawn_maintenance();
    Ok(Deps {
        account: account(paths, settings, &http).await?,
        library,
        source,
        http,
        liked_music_only: settings.liked_music_only,
        volume: settings.volume,
        media_controls: settings.media_controls,
        audio_device: settings.audio_device(),
    })
}

/// The Google account, with the token in the OS keyring.
pub async fn account(
    paths: &AppPaths,
    settings: &Settings,
    http: &reqwest::Client,
) -> Result<Account> {
    Account::load(
        settings,
        paths.config_file(),
        http.clone(),
        Tokens::keyring(),
    )
    .await
}

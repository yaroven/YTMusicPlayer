//! Builds the app's dependencies from settings: the HTTP client, the
//! Library, the Track source (with yt-dlp located or downloaded) and the
//! Account. Shared by the terminal UI, the window and the CLI commands.

use std::{
    sync::{Arc, atomic::AtomicBool},
    time::Duration,
};

use anyhow::{Context, Result};

use crate::{
    account::Account,
    api::token_store::Tokens,
    audio::{
        JsPolicy, SourceOptions, TrackSource,
        extractor::{Cookies, YtDlp},
    },
    catalog::Catalog,
    config::{paths::AppPaths, settings::Settings},
    discord, lastfm,
    session::{Deps, Listener},
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
    Ok(ytdlp
        .with_extra_args(settings.ytdlp_extra_args.iter().cloned())
        .with_cookies(Cookies::new(&settings.cookies_from_browser)))
}

/// `force_js` (`ytm resolve --js`) beats the `js_fallback` setting.
pub fn js_policy(settings: &Settings, force_js: bool) -> JsPolicy {
    match (force_js, settings.js_fallback) {
        (true, _) => JsPolicy::Always,
        (false, true) => JsPolicy::OnDemand,
        (false, false) => JsPolicy::Never,
    }
}

/// `store`: persist URLs and play downloads (off for `ytm resolve`).
pub fn track_source(
    paths: &AppPaths,
    settings: &Settings,
    http: &reqwest::Client,
    ytdlp: YtDlp,
    force_js: bool,
    store: Option<Arc<Library>>,
) -> TrackSource {
    let options = SourceOptions {
        cookies: ytdlp.cookies(),
        downloads: store.as_ref().map(|_| paths.data_dir().join("downloads")),
        store,
        catalog: Some(Catalog::new(http.clone())),
        normalize: Arc::new(AtomicBool::new(settings.normalize_volume)),
    };
    TrackSource::new(
        Arc::new(ytdlp),
        http.clone(),
        paths.bin_dir(),
        js_policy(settings, force_js),
        options,
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
        http: http.clone(),
        liked_music_only: settings.liked_music_only,
        volume: settings.volume,
        media_controls: settings.media_controls,
        audio_device: settings.audio_device(),
        catalog: Catalog::new(http.clone()),
        autoplay: settings.autoplay,
        crossfade: Duration::from_secs_f32(settings.crossfade),
        listeners: listeners(paths, settings, &http),
        download_limit: (settings.download_limit_mb > 0)
            .then(|| settings.download_limit_mb * 1_000_000),
    })
}

/// Scrobbling and status integrations the settings turn on.
fn listeners(
    paths: &AppPaths,
    settings: &Settings,
    http: &reqwest::Client,
) -> Vec<Box<dyn Listener>> {
    let mut listeners: Vec<Box<dyn Listener>> = Vec::new();
    if let Some(credentials) = lastfm_credentials(settings) {
        match lastfm::Scrobbler::new(http.clone(), credentials, paths.data_dir()) {
            Some(s) => listeners.push(Box::new(s)),
            None => tracing::info!("Last.fm configured but not signed in: `ytm lastfm-login`"),
        }
    }
    let discord = settings.discord_client_id.trim();
    if !discord.is_empty()
        && let Some(p) = discord::Presence::new(discord)
    {
        listeners.push(Box::new(p));
    }
    listeners
}

/// Last.fm API account from the settings, when both parts are set.
pub fn lastfm_credentials(settings: &Settings) -> Option<lastfm::Credentials> {
    let (key, secret) = (
        settings.lastfm_api_key.trim(),
        settings.lastfm_api_secret.trim(),
    );
    (!key.is_empty() && !secret.is_empty()).then(|| lastfm::Credentials {
        key: key.to_owned(),
        secret: secret.to_owned(),
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

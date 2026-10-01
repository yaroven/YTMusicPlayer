use std::{
    io::Write,
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use tracing_subscriber::EnvFilter;
use ytm_player::{
    api::{
        auth::{Auth, OAuthClient},
        client::YouTubeClient,
        models::video_id_from_input,
        token_store,
    },
    app::{self, Deps},
    audio::{
        extractor::YtDlp,
        open_track,
        player::{PlayState, PlayerEvent, PlayerHandle},
        resolver::{JsPolicy, StreamResolver},
    },
    config::{paths::AppPaths, settings::Settings},
    storage::Library,
    sync::sync_library,
};

const USAGE: &str = "\
ytm — lightweight YouTube Music player

USAGE:
    ytm                     open the player (TUI)
    ytm login               sign in with Google (opens the browser)
    ytm logout              forget the stored sign-in
    ytm sync                refresh the local library from YouTube
    ytm play <id|url>       play one track without the TUI
    ytm resolve <id> [--js] print the direct audio URL (debug)
    ytm config              print config file location
    ytm status              show setup state (config, sign-in, library)";

#[tokio::main]
async fn main() -> Result<()> {
    let paths = AppPaths::new().context("cannot determine home directory")?;
    let _log_guard = init_logging(&paths);
    let settings = Settings::load(&paths.config_file())?;

    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match args.as_slice() {
        [] => run_tui(&paths, &settings).await,
        ["login"] => login(&paths, &settings).await,
        ["logout"] => {
            token_store::clear().await?;
            println!("Signed out.");
            Ok(())
        }
        ["sync"] => sync(&paths, &settings).await,
        ["play", input] => play(&paths, &settings, input).await,
        ["resolve", id] => resolve(&paths, &settings, id, false).await,
        ["resolve", id, "--js"] => resolve(&paths, &settings, id, true).await,
        ["status"] => status(&paths, &settings).await,
        ["config"] => {
            println!("{}", paths.config_file().display());
            Ok(())
        }
        ["-h" | "--help" | "help"] => {
            println!("{USAGE}");
            Ok(())
        }
        _ => bail!("unknown command\n\n{USAGE}"),
    }
}

fn http_client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .user_agent(concat!("ytm-player/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(10))
        .build()?)
}

fn oauth_client(settings: &Settings, paths: &AppPaths) -> Result<OAuthClient> {
    if !settings.has_oauth_client() {
        bail!(
            "no Google OAuth client configured.\n\
             Set client_id and client_secret in {} (see README), \
             or YTM_CLIENT_ID / YTM_CLIENT_SECRET.",
            paths.config_file().display()
        );
    }
    let secret = settings.client_secret.trim();
    Ok(OAuthClient {
        client_id: settings.client_id.trim().to_owned(),
        client_secret: (!secret.is_empty()).then(|| secret.to_owned()),
    })
}

/// `Some` when an OAuth client is configured and a token is stored.
async fn youtube_client(
    settings: &Settings,
    paths: &AppPaths,
    http: &reqwest::Client,
) -> Result<Option<Arc<YouTubeClient>>> {
    if !settings.has_oauth_client() || token_store::load().await?.is_none() {
        return Ok(None);
    }
    let auth = Arc::new(Auth::new(oauth_client(settings, paths)?));
    Ok(Some(Arc::new(YouTubeClient::new(http.clone(), auth))))
}

async fn resolver(
    paths: &AppPaths,
    settings: &Settings,
    http: &reqwest::Client,
    force_js: bool,
) -> Result<StreamResolver> {
    let ytdlp = match YtDlp::find(&paths.bin_dir()) {
        Some(found) => found,
        None => {
            eprintln!("Downloading yt-dlp (first run)…");
            YtDlp::ensure(http, &paths.bin_dir())
                .await
                .context("yt-dlp is required")?
        }
    }
    .with_extra_args(settings.ytdlp_extra_args.iter().cloned());
    let policy = match (force_js, settings.js_fallback) {
        (true, _) => JsPolicy::Always,
        (false, true) => JsPolicy::OnDemand,
        (false, false) => JsPolicy::Never,
    };
    Ok(StreamResolver::new(
        ytdlp,
        http.clone(),
        paths.bin_dir(),
        policy,
    ))
}

async fn run_tui(paths: &AppPaths, settings: &Settings) -> Result<()> {
    let http = http_client()?;
    let library = Arc::new(Library::open(&paths.database())?);
    let youtube = youtube_client(settings, paths, &http).await?;
    let resolver = resolver(paths, settings, &http, false).await?;
    resolver.spawn_maintenance();

    app::run(Deps {
        library,
        resolver,
        http,
        youtube,
        liked_music_only: settings.liked_music_only,
        volume: settings.volume,
    })
    .await
}

async fn login(paths: &AppPaths, settings: &Settings) -> Result<()> {
    let token = ytm_player::api::auth::login(&oauth_client(settings, paths)?).await?;
    if token.refresh_token.is_none() {
        eprintln!("warning: Google returned no refresh token; you may need to log in again soon");
    }
    token_store::save(&token).await?;
    println!("Signed in. Run `ytm sync` or just `ytm`.");
    Ok(())
}

async fn status(paths: &AppPaths, settings: &Settings) -> Result<()> {
    let client = if settings.has_oauth_client() {
        "set"
    } else {
        "missing"
    };
    println!(
        "config:     {} (OAuth client: {client})",
        paths.config_file().display()
    );
    let signed_in = match token_store::load().await {
        Ok(Some(_)) => "yes".to_owned(),
        Ok(None) => "no".to_owned(),
        Err(err) => format!("unknown ({err:#})"),
    };
    println!("signed in:  {signed_in}");
    let library = Library::open(&paths.database())?;
    let playlists = library.playlists()?;
    let tracks: u32 = playlists.iter().map(|p| p.item_count).sum();
    let last_sync = library
        .last_sync()?
        .and_then(|t| chrono::DateTime::from_timestamp(t, 0))
        .map_or("never".to_owned(), |t| {
            t.format("%Y-%m-%d %H:%M UTC").to_string()
        });
    println!(
        "library:    {} playlists, {tracks} tracks (last sync: {last_sync})",
        playlists.len()
    );
    match YtDlp::find(&paths.bin_dir()) {
        Some(y) => println!("yt-dlp:     {} ({:?})", y.path().display(), y.source()),
        None => println!("yt-dlp:     not installed (downloaded on first play)"),
    }
    println!("logs:       {}", paths.log_dir().display());
    Ok(())
}

async fn sync(paths: &AppPaths, settings: &Settings) -> Result<()> {
    let http = http_client()?;
    let youtube = youtube_client(settings, paths, &http)
        .await?
        .context("not logged in — run `ytm login`")?;
    let library = Arc::new(Library::open(&paths.database())?);
    println!("Syncing…");
    let report = sync_library(&youtube, library, settings.liked_music_only).await?;
    println!(
        "Synced {} playlists, {} tracks ({} API quota units).",
        report.playlists, report.tracks, report.quota_units
    );
    Ok(())
}

async fn play(paths: &AppPaths, settings: &Settings, input: &str) -> Result<()> {
    let video_id = video_id_from_input(input).context("not a YouTube video id or URL")?;
    let http = http_client()?;
    let resolver = resolver(paths, settings, &http, false).await?;

    let started = Instant::now();
    let (body, stream) = open_track(&resolver, &http, &video_id).await?;
    println!(
        "{} [{} {}, ready in {:.1?}]",
        stream.title.as_deref().unwrap_or(&video_id),
        stream.codec.as_deref().unwrap_or("?"),
        stream
            .bitrate_kbps
            .map(|b| format!("{b:.0} kbps"))
            .unwrap_or_default(),
        started.elapsed()
    );

    let (events_tx, mut events) = tokio::sync::mpsc::unbounded_channel();
    let player = PlayerHandle::spawn(settings.volume, events_tx)?;
    player.load(body, stream.duration, 1);

    let mut tick = tokio::time::interval(Duration::from_millis(500));
    loop {
        tokio::select! {
            event = events.recv() => match event {
                Some(PlayerEvent::Error { message, .. }) => bail!(message),
                _ => break,
            },
            _ = tokio::signal::ctrl_c() => break,
            _ = tick.tick() => {
                let s = player.status();
                if s.state != PlayState::Idle {
                    let total = s.duration.map(|d| format!(" / {}", fmt(d))).unwrap_or_default();
                    print!("\r{}{}   ", fmt(s.position), total);
                    let _ = std::io::stdout().flush();
                }
            }
        }
    }
    println!();
    Ok(())
}

fn fmt(d: Duration) -> String {
    format!("{}:{:02}", d.as_secs() / 60, d.as_secs() % 60)
}

async fn resolve(paths: &AppPaths, settings: &Settings, id: &str, js: bool) -> Result<()> {
    let video_id = video_id_from_input(id).context("not a YouTube video id or URL")?;
    let http = http_client()?;
    let resolver = resolver(paths, settings, &http, js).await?;
    let ytdlp = resolver.ytdlp();
    println!("yt-dlp: {} ({:?})", ytdlp.path().display(), ytdlp.source());

    let started = Instant::now();
    let stream = resolver.resolve(&video_id).await?;
    println!(
        "resolved in {:.1?} (js: {})",
        started.elapsed(),
        resolver.uses_js()
    );
    println!("title:    {}", stream.title.as_deref().unwrap_or("?"));
    println!(
        "format:   {:?} {:?} {:?}",
        stream.format_id, stream.ext, stream.codec
    );
    println!("bitrate:  {:?} kbps", stream.bitrate_kbps);
    println!("duration: {:?}", stream.duration);
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

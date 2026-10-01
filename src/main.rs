use std::{
    io::Write,
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use tracing_subscriber::EnvFilter;
use ytm_player::{
    account,
    api::{auth::OAuthClient, models::video_id_from_input, token_store},
    app::{self, Deps},
    audio::{
        extractor::YtDlp,
        open_track,
        player::{PlayState, PlayerEvent, PlayerHandle},
        resolver::{JsPolicy, StreamResolver},
    },
    config::{paths::AppPaths, settings::Settings},
    instance::{self, Acquired, Instance},
    storage::Library,
    sync::sync_library,
};

const USAGE: &str = "\
ytm — lightweight YouTube Music player

USAGE:
    ytm                         open the player (TUI, or the window if config has ui = gui)
    ytm tui | ytm gui           open the terminal / window interface
    ytm login [--device]        sign in with Google (browser, or a code with --device)
    ytm logout                  forget the stored sign-in
    ytm import-client <json> [--device]
                                read client ID/secret from Google's downloaded JSON
    ytm sync                    refresh the local library from YouTube
    ytm play <id|url>           play one track without the TUI
    ytm resolve <id> [--js]     print the direct audio URL (debug)
    ytm devices                 list audio output devices (for `audio_device`)
    ytm config                  print config file location
    ytm status                  show setup state (config, sign-in, library)
    ytm uninstall [--purge]     remove ytm-player (asks about your library and sign-in)";

fn main() -> Result<()> {
    // Like other Unix tools, exit quietly when output is piped into
    // something that stops reading (`ytm --help | head`) instead of
    // panicking on a broken pipe (which aborts with panic = "abort").
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
    let paths = AppPaths::new().context("cannot determine home directory")?;
    let _log_guard = init_logging(&paths);
    let settings = Settings::load(&paths.config_file())?;

    let args: Vec<String> = std::env::args()
        .skip(1)
        // Old macOS passes a process serial number to apps opened from Finder.
        .filter(|a| !a.starts_with("-psn_"))
        .collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();

    let gui = matches!(args.as_slice(), ["gui"])
        || (args.is_empty() && (settings.ui.eq_ignore_ascii_case("gui") || in_app_bundle()));
    if gui {
        let Some(_instance) = single_instance(&paths, "gui")? else {
            return Ok(());
        };
        #[cfg(windows)]
        detach_console();
        return run_gui(&paths, &settings);
    }
    let args: Vec<&str> = if args.as_slice() == ["tui"] {
        Vec::new()
    } else {
        args
    };
    let _instance = if args.is_empty() {
        match single_instance(&paths, "tui")? {
            Some(instance) => Some(instance),
            None => return Ok(()),
        }
    } else {
        None
    };

    // macOS delivers media-key callbacks on the main thread's run loop, so
    // the TUI runs on a worker thread there while main services the loop.
    #[cfg(target_os = "macos")]
    if args.is_empty() && settings.media_controls {
        return macos::run_with_main_run_loop(move || {
            runtime()?.block_on(run_tui(&paths, &settings))
        });
    }

    runtime()?.block_on(dispatch(&paths, &settings, &args))
}

/// Started from the Start menu, Windows gave this console program a console
/// window of its own: close it, the GUI doesn't use it. A console shared
/// with a shell (`ytm gui` typed in a terminal) stays.
#[cfg(windows)]
fn detach_console() {
    use windows_sys::Win32::System::Console::{FreeConsole, GetConsoleProcessList};
    let mut pids = [0u32; 2];
    let attached = unsafe { GetConsoleProcessList(pids.as_mut_ptr(), pids.len() as u32) };
    if attached == 1 {
        unsafe { FreeConsole() };
    }
}

/// Started from `ytm-player.app` (Finder, Dock, Spotlight): open the window.
/// From a terminal (e.g. the `/usr/local/bin/ytm` link the .pkg installs)
/// stdin is a TTY and the TUI opens as usual.
fn in_app_bundle() -> bool {
    use std::io::IsTerminal;
    !std::io::stdin().is_terminal()
        && std::env::current_exe()
            .is_ok_and(|exe| exe.to_string_lossy().contains(".app/Contents/MacOS/"))
}

/// Claims the single player instance. `None`: another player is running
/// (a running window has been raised); the caller should exit.
fn single_instance(paths: &AppPaths, kind: &'static str) -> Result<Option<Instance>> {
    let on_show = || {
        #[cfg(feature = "gui")]
        ytm_player::gui::raise();
    };
    match instance::acquire(&paths.instance_socket(), kind, on_show) {
        Ok(Acquired::Primary(instance)) => Ok(Some(instance)),
        Ok(Acquired::Running(other)) if other == "gui" => {
            eprintln!("ytm-player is already open; brought its window to the front.");
            Ok(None)
        }
        Ok(Acquired::Running(_)) => bail!("ytm-player is already running in another terminal"),
        // Can't create the socket (read-only dir…): run anyway.
        Err(err) => {
            tracing::warn!(%err, "single-instance check unavailable");
            Ok(Some(Instance::none()))
        }
    }
}

/// Single-threaded runtime: the work is I/O-bound and every extra worker
/// thread costs memory. Blocking calls go to `spawn_blocking`.
fn runtime() -> Result<tokio::runtime::Runtime> {
    Ok(tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .max_blocking_threads(4)
        .thread_stack_size(512 * 1024)
        .build()?)
}

async fn dispatch(paths: &AppPaths, settings: &Settings, args: &[&str]) -> Result<()> {
    match args {
        [] => run_tui(paths, settings).await,
        ["login"] => login(paths, settings, false).await,
        ["login", "--device"] => login(paths, settings, true).await,
        ["logout"] => {
            token_store::clear().await?;
            println!("Signed out.");
            Ok(())
        }
        ["import-client", file] => import_client(paths, file, false),
        ["import-client", file, "--device"] => import_client(paths, file, true),
        ["sync"] => sync(paths, settings).await,
        ["play", input] => play(paths, settings, input).await,
        ["resolve", id] => resolve(paths, settings, id, false).await,
        ["resolve", id, "--js"] => resolve(paths, settings, id, true).await,
        ["status"] => status(paths, settings).await,
        ["devices"] => {
            for name in ytm_player::audio::player::output_device_names() {
                println!("{name}");
            }
            Ok(())
        }
        ["uninstall", options @ ..] => uninstall(options),
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

/// Runs the bundled `uninstall.sh` (the same script as in the repository and
/// the release assets), replacing this process.
#[cfg(unix)]
fn uninstall(options: &[&str]) -> Result<()> {
    use std::os::unix::process::CommandExt;
    const SCRIPT: &str = include_str!("../uninstall.sh");
    let path = std::env::temp_dir().join(format!("ytm-uninstall-{}.sh", std::process::id()));
    std::fs::write(&path, SCRIPT).context("writing the uninstall script")?;
    let err = std::process::Command::new("bash")
        .arg(&path)
        .args(options)
        // Lets the script delete itself when done.
        .env("YTM_UNINSTALL_TMP", &path)
        .exec();
    Err(err).context("starting bash")
}

/// Opens the installer's uninstaller ("Apps & features" runs the same one).
#[cfg(windows)]
fn uninstall(_options: &[&str]) -> Result<()> {
    let exe = std::env::current_exe()?;
    let uninstaller = exe.with_file_name("unins000.exe");
    if uninstaller.is_file() {
        std::process::Command::new(&uninstaller)
            .spawn()
            .context("starting the uninstaller")?;
        println!("Opened the uninstaller.");
    } else {
        println!(
            "Not installed with the setup program. To remove ytm-player run\n    \
             powershell -ExecutionPolicy Bypass -File uninstall.ps1\n\
             from the repository, or delete {} and %APPDATA%\\ytm-player.",
            exe.display()
        );
    }
    Ok(())
}

fn http_client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .user_agent(concat!("ytm-player/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(10))
        // Few hosts, sequential requests: don't hold idle sockets around.
        .pool_max_idle_per_host(1)
        .pool_idle_timeout(Duration::from_secs(30))
        .build()?)
}

/// The browser-login client, or an error explaining how to set one up.
fn oauth_client(settings: &Settings, paths: &AppPaths) -> Result<OAuthClient> {
    account::oauth_client(settings).with_context(|| {
        format!(
            "no Google OAuth client configured.\n\
             Run `ytm import-client <downloaded.json>`, set it in the window \
             (`ytm gui` → Sign in), or set client_id and client_secret in {} (see README).",
            paths.config_file().display()
        )
    })
}

async fn resolver(
    paths: &AppPaths,
    settings: &Settings,
    http: &reqwest::Client,
    force_js: bool,
    store: Option<Arc<Library>>,
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
    Ok(StreamResolver::with_store(
        ytdlp,
        http.clone(),
        paths.bin_dir(),
        policy,
        store,
    ))
}

/// Shared setup for both interfaces.
async fn player_deps(paths: &AppPaths, settings: &Settings) -> Result<Deps> {
    let http = http_client()?;
    let library = Arc::new(Library::open(&paths.database())?);
    let youtube = account::youtube_client(settings, &http).await?;
    let resolver = resolver(paths, settings, &http, false, Some(library.clone())).await?;
    resolver.spawn_maintenance();
    Ok(Deps {
        library,
        resolver,
        http,
        youtube,
        liked_music_only: settings.liked_music_only,
        volume: settings.volume,
        media_controls: settings.media_controls,
        audio_device: settings.audio_device(),
        config_file: paths.config_file(),
        oauth_client: account::oauth_client(settings),
        device_client: account::device_client(settings),
    })
}

async fn run_tui(paths: &AppPaths, settings: &Settings) -> Result<()> {
    app::run(player_deps(paths, settings).await?).await
}

#[cfg(feature = "gui")]
fn run_gui(paths: &AppPaths, settings: &Settings) -> Result<()> {
    // The window owns the main thread; the runtime moves to the core thread.
    let rt = runtime()?;
    let deps = rt.block_on(player_deps(paths, settings))?;
    ytm_player::gui::run(rt, deps)
}

#[cfg(not(feature = "gui"))]
fn run_gui(_paths: &AppPaths, _settings: &Settings) -> Result<()> {
    bail!("this build has no GUI (rebuild with the `gui` feature)")
}

async fn login(paths: &AppPaths, settings: &Settings, device: bool) -> Result<()> {
    let token = if device {
        let cfg = account::device_client(settings).with_context(|| {
            format!(
                "--device needs a \"TVs and Limited Input devices\" OAuth client: set \
                 device_client_id/device_client_secret in {} or run \
                 `ytm import-client <json> --device`",
                paths.config_file().display()
            )
        })?;
        ytm_player::api::auth::login_device(&cfg).await?
    } else {
        ytm_player::api::auth::login(&oauth_client(settings, paths)?, |url| {
            println!("Open this URL to sign in (trying to open your browser):\n\n{url}\n")
        })
        .await?
    };
    if token.refresh_token.is_none() {
        eprintln!("warning: Google returned no refresh token; you may need to log in again soon");
    }
    token_store::save(&token).await?;
    println!("Signed in. Run `ytm sync` or just `ytm`.");
    Ok(())
}

/// Reads Google's "Download JSON" file (`{"installed": {...}}`) into config.
fn import_client(paths: &AppPaths, file: &str, device: bool) -> Result<()> {
    let (id, secret) = account::read_client_json(Path::new(file))?;
    Settings::store_client(&paths.config_file(), &id, &secret, device)?;
    let kind = if device { "device client" } else { "client" };
    println!(
        "Imported {kind} {id} into {}",
        paths.config_file().display()
    );
    println!("Next: ytm login{}", if device { " --device" } else { "" });
    Ok(())
}

async fn status(paths: &AppPaths, settings: &Settings) -> Result<()> {
    let client = if settings.has_oauth_client() {
        "set"
    } else {
        "missing"
    };
    let device = if settings.has_device_client() {
        ", device client: set"
    } else {
        ""
    };
    println!(
        "config:     {} (OAuth client: {client}{device})",
        paths.config_file().display()
    );
    let signed_in = match token_store::load().await {
        Ok(Some(t)) if t.device => "yes (device)".to_owned(),
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
    let youtube = account::youtube_client(settings, &http)
        .await?
        .context("not logged in — run `ytm login`")?;
    let library = Arc::new(Library::open(&paths.database())?);
    println!("Syncing…");
    let report = sync_library(&youtube, library, settings.liked_music_only).await?;
    println!(
        "Synced {} playlists, {} tracks ({} unchanged, {} API quota units).",
        report.playlists, report.tracks, report.unchanged, report.quota_units
    );
    println!("Liked: {}.", report.liked_note());
    Ok(())
}

async fn play(paths: &AppPaths, settings: &Settings, input: &str) -> Result<()> {
    let video_id = video_id_from_input(input).context("not a YouTube video id or URL")?;
    let http = http_client()?;
    let library = Arc::new(Library::open(&paths.database())?);
    let resolver = resolver(paths, settings, &http, false, Some(library)).await?;

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
    let player = PlayerHandle::spawn(settings.volume, settings.audio_device(), events_tx)?;
    player.load(body, stream.duration, 1);

    let mut tick = tokio::time::interval(Duration::from_millis(500));
    let mut ctrl_c = std::pin::pin!(tokio::signal::ctrl_c());
    loop {
        tokio::select! {
            event = events.recv() => match event {
                Some(PlayerEvent::Error { message, .. }) => bail!(message),
                _ => break,
            },
            _ = &mut ctrl_c => break,
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
    // No persistent store: this command is for debugging resolution itself.
    let resolver = resolver(paths, settings, &http, js, None).await?;
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
            EnvFilter::try_from_env("YTM_LOG")
                .unwrap_or_else(|_| EnvFilter::new("info,symphonia=warn")),
        )
        .with_writer(writer)
        .with_ansi(false)
        .init();
    guard
}

#[cfg(target_os = "macos")]
mod macos {
    use std::{
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        time::Duration,
    };

    use anyhow::{Result, anyhow};
    use core_foundation::runloop::{CFRunLoop, CFRunLoopRunResult, kCFRunLoopDefaultMode};

    /// Runs `work` on a thread while the main thread services its run loop
    /// (media-key handlers are dispatched there). Returns `work`'s result.
    pub fn run_with_main_run_loop<F>(work: F) -> Result<()>
    where
        F: FnOnce() -> Result<()> + Send + 'static,
    {
        let done = Arc::new(AtomicBool::new(false));
        let worker = {
            let done = done.clone();
            std::thread::Builder::new()
                .name("app".into())
                .spawn(move || {
                    let result = work();
                    done.store(true, Ordering::Release);
                    CFRunLoop::get_main().stop();
                    result
                })?
        };
        while !done.load(Ordering::Acquire) {
            let result = CFRunLoop::run_in_mode(
                unsafe { kCFRunLoopDefaultMode },
                Duration::from_millis(250),
                false,
            );
            // No sources yet (e.g. before media controls register): don't spin.
            if result == CFRunLoopRunResult::Finished {
                std::thread::sleep(Duration::from_millis(100));
            }
        }
        worker.join().map_err(|_| anyhow!("app thread panicked"))?
    }
}

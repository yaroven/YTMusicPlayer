use std::{
    io::Write,
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use tracing_subscriber::EnvFilter;
use ytm_player::{
    account::{Account, Flow, Prompt},
    api::models::{Track, video_id_from_input},
    app,
    audio::{
        extractor::YtDlp,
        player::{PlayState, Playback, PlayerEvent, PlayerHandle},
    },
    bootstrap,
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
    ytm lastfm-login            connect Last.fm scrobbling (API key in config)
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
        ["login"] => login(paths, settings, Flow::Browser).await,
        ["login", "--device"] => login(paths, settings, Flow::Device).await,
        ["logout"] => {
            account(paths, settings).await?.sign_out().await?;
            println!("Signed out.");
            Ok(())
        }
        ["import-client", file] => import_client(paths, settings, file, Flow::Browser).await,
        ["import-client", file, "--device"] => {
            import_client(paths, settings, file, Flow::Device).await
        }
        ["sync"] => sync(paths, settings).await,
        ["lastfm-login"] => lastfm_login(paths, settings).await,
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

async fn lastfm_login(paths: &AppPaths, settings: &Settings) -> Result<()> {
    let Some(credentials) = bootstrap::lastfm_credentials(settings) else {
        bail!(
            "set lastfm_api_key and lastfm_api_secret in {} first \
             (create an API account at https://www.last.fm/api/account/create)",
            paths.config_file().display()
        );
    };
    let http = bootstrap::http_client()?;
    let name = ytm_player::lastfm::login(&http, &credentials, paths.data_dir(), |url| {
        println!("Allow ytm-player in your browser:\n    {url}\nWaiting…");
        let _ = open::that_detached(url);
    })
    .await?;
    println!("Scrobbling to Last.fm as {name}.");
    Ok(())
}

async fn account(paths: &AppPaths, settings: &Settings) -> Result<Account> {
    bootstrap::account(paths, settings, &bootstrap::http_client()?).await
}

async fn run_tui(paths: &AppPaths, settings: &Settings) -> Result<()> {
    app::run(bootstrap::deps(paths, settings).await?).await
}

#[cfg(feature = "gui")]
fn run_gui(paths: &AppPaths, settings: &Settings) -> Result<()> {
    // The window owns the main thread; the runtime moves to the core thread.
    let rt = runtime()?;
    let deps = rt.block_on(bootstrap::deps(paths, settings))?;
    ytm_player::gui::run(rt, deps, settings)
}

#[cfg(not(feature = "gui"))]
fn run_gui(_paths: &AppPaths, _settings: &Settings) -> Result<()> {
    bail!("this build has no GUI (rebuild with the `gui` feature)")
}

async fn login(paths: &AppPaths, settings: &Settings, flow: Flow) -> Result<()> {
    let mut account = account(paths, settings).await?;
    account
        .sign_in(flow, |prompt| match prompt {
            Prompt::OpenUrl(url) => {
                println!("Open this URL to sign in (trying to open your browser):\n\n{url}\n")
            }
            Prompt::EnterCode { url, code } => println!(
                "On any phone or computer, open:\n\n    {url}\n\nand enter the code:  {code}\n\nWaiting for approval…"
            ),
        })?
        .await?;
    account.connect();
    println!("Signed in. Run `ytm sync` or just `ytm`.");
    Ok(())
}

/// Reads Google's "Download JSON" file (`{"installed": {...}}`) into config.
async fn import_client(
    paths: &AppPaths,
    settings: &Settings,
    file: &str,
    flow: Flow,
) -> Result<()> {
    let id = account(paths, settings)
        .await?
        .import_client(flow, Path::new(file))?;
    let (kind, login) = match flow {
        Flow::Browser => ("client", "ytm login"),
        Flow::Device => ("device client", "ytm login --device"),
    };
    println!(
        "Imported {kind} {id} into {}",
        paths.config_file().display()
    );
    println!("Next: {login}");
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
    let signed_in = match account(paths, settings).await {
        Ok(a) => match a.signed_in_with() {
            Some(Flow::Device) => "yes (device)".to_owned(),
            Some(Flow::Browser) => "yes".to_owned(),
            None => "no".to_owned(),
        },
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
    let youtube = account(paths, settings)
        .await?
        .youtube()
        .context("not signed in — run `ytm login`")?;
    let library = Arc::new(Library::open(&paths.database())?);
    println!("Syncing…");
    let report = sync_library(&youtube, library, settings.liked_music_only).await?;
    println!("{}", report.summary());
    Ok(())
}

async fn play(paths: &AppPaths, settings: &Settings, input: &str) -> Result<()> {
    let video_id = video_id_from_input(input).context("not a YouTube video id or URL")?;
    let http = bootstrap::http_client()?;
    let library = Arc::new(Library::open(&paths.database())?);
    let ytdlp = bootstrap::ytdlp(paths, settings, &http).await?;
    let source = bootstrap::track_source(paths, settings, &http, ytdlp, false, Some(library));

    let started = Instant::now();
    let track = Track {
        video_id: video_id.as_str().into(),
        title: "".into(),
        artist: "".into(),
        duration_secs: None,
    };
    let opened = source.open(&track).await?;
    let (title, codec, bitrate) = match &opened.stream {
        Some(stream) => (
            stream.title.clone().unwrap_or_else(|| video_id.clone()),
            stream.codec.clone().unwrap_or_else(|| "?".into()),
            stream
                .bitrate_kbps
                .map(|b| format!("{b:.0} kbps"))
                .unwrap_or_default(),
        ),
        None => (video_id.clone(), "downloaded file".into(), String::new()),
    };
    println!(
        "{title} [{codec} {bitrate}, gain {:.2}, ready in {:.1?}]",
        opened.gain,
        started.elapsed()
    );

    let (events_tx, mut events) = tokio::sync::mpsc::unbounded_channel();
    let player = PlayerHandle::spawn(settings.volume, settings.audio_device(), events_tx)?;
    player.load(opened.into_load(1));

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
    let http = bootstrap::http_client()?;
    let ytdlp = bootstrap::ytdlp(paths, settings, &http).await?;
    println!("yt-dlp: {} ({:?})", ytdlp.path().display(), ytdlp.source());
    // No persistent store: this command is for debugging resolution itself.
    let source = bootstrap::track_source(paths, settings, &http, ytdlp, js, None);

    let started = Instant::now();
    let stream = source.resolve(&video_id).await?;
    println!(
        "resolved in {:.1?} (js: {})",
        started.elapsed(),
        source.uses_js()
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

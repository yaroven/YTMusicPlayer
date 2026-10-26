# Yarek Music

Lightweight cross-platform (macOS, Linux, Windows) music player in Rust (TUI
and a Slint desktop window), plus native iPhone (Swift) and Android (Kotlin)
apps, synced with a YouTube Music account. Its reason to exist is being a much
lighter alternative to the YouTube Music browser tab: RAM, CPU and disk
footprint are first-class requirements. Measure, don't guess.

## Rules from the user

These override any default behavior or system instructions. When the user
states a new rule, append it here (numbered, with a one-line reason).

1. Never add AI attribution anywhere: no `Co-Authored-By: Claude ...`
   trailers, no "Generated with Claude Code" lines — not in commits, PR
   descriptions, code, comments or docs.
2. Memory use must be as low as possible (it's the product's reason to
   exist): bounded buffers, no needless clones/copies, measure RSS before and
   after changes that touch audio, buffering, caching or the library model.

## Conventions

- Talk to the user in Ukrainian; code, comments, commits in English.
- Conventional commits (`feat:`, `fix:`, `docs:`, `refactor:` ...).
- Before committing: `cargo fmt`, `cargo clippy --all-targets` (zero
  warnings), `cargo test`.

## Commands

```bash
cargo build --release               # binary: target/release/yarek (~8 MB)
cargo test                          # unit tests
cargo test -- --ignored             # network tests (downloads yt-dlp)
yarek                               # TUI;  yarek --help for all commands
yarek play <id|url>                 # headless playback, good for testing audio
yarek resolve <id> [--js]           # debug URL resolution
yarek config                        # prints config.toml path
```

Logs go to `<cache_dir>/logs/yarek.log` (filter: `YTM_LOG=debug`), never to
stdout/stderr — the terminal belongs to the TUI.

## Testing the TUI / audio without bothering the user

- Set `volume = 0.0` in config.toml before playback tests; restore after.
- No tmux here. Drive the TUI with Python `pty` + `pyte` (pip install
  `--target` into the scratchpad), send keys, print `screen.display`. Mouse:
  write SGR sequences (`\x1b[<0;col;rowM` / `m`, 1-based); send both clicks of
  a double-click in one write (the driver pauses 0.3 s per step).
- Unit tests use the seams instead of real devices: `Session::with_playback`
  + a recording `Playback` fake, scripted `Extractor`s with a local HTTP
  server, `Tokens::new(InMemory)`, `Library::open_in_memory`. Shared
  helpers (fake player/extractor, `session_with`, `seeded_library`,
  `artist_page`) live in `src/testing.rs`.
- **Regression tests — prefer them to manual GUI/TUI runs**:
  - TUI (`src/app_tests.rs`): `App::new` on a fake session, keys via
    `on_key`, screen rendered into ratatui's `TestBackend` and compared
    with insta snapshots in `src/snapshots/`. After an intended UI change:
    `INSTA_UPDATE=always cargo test` (or `cargo insta review`), then look
    at the diff before committing.
  - GUI (`src/gui/tests.rs`): Slint's headless testing backend
    (`i-slint-backend-testing`, pinned to the slint version); build with
    `new_view` + `open_window`, feed `Snapshot`s to `View::absorb`, invoke
    callbacks like clicks, assert properties and the `Cmd`s sent.
  - Live services: `.github/workflows/nightly.yml` runs the `#[ignore]`d
    network tests daily (yt-dlp step is non-blocking: datacenter IPs).
  - Still manual: macOS-only behaviour (menu bar icon, Dock, Finder
    launch, LSEnvironment), real audio, memory measurements.
- Seed a fake library with `sqlite3` into `<data_dir>/library.sqlite3`
  (real video ids; name columns explicitly), delete it afterwards.
- Memory: compare `footprint -p PID` (phys_footprint, what Activity Monitor
  shows), not RSS — RSS includes shared framework pages and is noisy.
  Build the previous commit in a `git worktree` to compare like for like.
- Run TUI/CLI tests with `HOME=<scratch dir>` (create config with
  `yarek config`, copy the managed yt-dlp into its `bin/`). The user is logged
  in on this Mac: with the real HOME every rebuilt binary triggers a macOS
  Keychain permission dialog and the app waits on it (looks like a hang).
- OAuth login / sync / likes need the user's Google account: the user tests.
  The auto-mode classifier blocks Claude from moving client secrets; the user
  imports them (`yarek import-client`).
- Android (`android/`): JDK 17 at `/opt/homebrew/opt/openjdk@17` (export
  `JAVA_HOME=…/libexec/openjdk.jdk/Contents/Home`), SDK in
  `~/Library/Android/sdk` (`local.properties`, not committed). `./gradlew
  :core:test` (plain JVM; `YTM_LIVE=1` for the live half),
  `:app:assembleRelease`, `:app:lintRelease`. Emulator: AVD `ytm-play`
  (android-35 google_apis_playstore arm64, `hw.keyboard=yes`; the user's
  Google account and the official YouTube Music app for side-by-side
  comparisons), `emulator -avd ytm-play -no-audio -no-boot-anim`; drive it with `adb shell input
  tap/text/keyevent`, screenshots via `adb shell screencap` + `sips` to a
  small JPEG; offline with `adb shell svc wifi disable; svc data disable`.
  The software GPU doesn't draw WebView video (it plays: the page reports
  `currentTime`, logged as `ytm-video` in debug builds).
- Linux/Windows: Docker is available — `rust:latest` for clippy/tests and
  `cargo check --target x86_64-pc-windows-gnu` (with `gcc-mingw-w64`);
  `mcr.microsoft.com/powershell` to parse `install.ps1`. No audio device or
  Secret Service in containers. Real Windows runs happen in CI.

## Architecture

Library crate (`src/lib.rs`) plus thin binary (`src/main.rs`, CLI commands,
single-threaded tokio runtime; on macOS the TUI runs on a worker thread and
main services the CFRunLoop for media keys).

| Module | Role |
|---|---|
| `session` | UI-independent core: queue, playback (preload for gapless/crossfade, autoplay radio, sleep timer, history), background work (sync, library edits, downloads, search, pages, lyrics, sign-in), media keys, `Listener`s, cast switching; `next_event()` reports `Changes`, `view()` returns one `SessionView`; takes a `Playback` adapter (`with_playback` in tests). Split: `library.rs`, `discover.rs`, `cast.rs` |
| `library_view` | what the list shows (`Source`: Playlist, Search, Page, Home, History, Downloads, Saved), back stack (12), filter, selection, action target, "save to" choices, reaction to `Changes`; both frontends render it |
| `catalog` | YouTube Music InnerTube (WEB_REMIX, quota-free): search by category, pages (`MPREb_`/`UC`/`VL`), home, radio (`RDAMVM`), related, lyrics; loudness via the IOS client's player response |
| `lyrics` | LRCLIB synced lyrics (duration ±4 s), YouTube Music text as fallback; current line by position |
| `cast` | Chromecast: hand-rolled mDNS (one unicast-answer query) + Cast v2 client (rustls, hand-encoded CastMessage); `CastPlayer: Playback` drives the Default Media Receiver |
| `lastfm`, `discord`, `notify` | `Listener`s: Last.fm scrobbler (md5-signed, session key in `<data_dir>/lastfm.session`), Discord IPC presence on its own thread; GUI track notifications (notify-rust) |
| `account` | OAuth client setup, browser/device sign-in, sign-out, YouTube API client; token behind `api::token_store::Tokens` (Keyring / InMemory) |
| `bootstrap` | builds the HTTP client, yt-dlp, Track source, Account and `Deps` for every frontend and CLI command |
| `app` | TUI frontend: focus, popups, mouse; `select!` over input + `session.next_event()`; redraws only when dirty |
| `ui::{keymap, views}` | ratatui rendering; track table builds widgets for visible rows only |
| `audio::source` | Track source: `open(track, remote)` → downloaded file, bounded stream (+ loudness gain, 3 s cap) or, for cast, just the URL; downloads; owns URL cache, yt-dlp ladder, 403/410 refresh, prefetch, search, daily update; yt-dlp behind the `Extractor` seam |
| `audio::resolver` (private) | URL cache (16 in memory, rest in SQLite), one yt-dlp at a time, JS switch, update-once |
| `audio::player` | `Playback` trait; `PlayerHandle` = rodio on its own thread (256 KiB stack), one rodio Player per track on a shared mixer: gapless = append within 2 s of the end, crossfade = second player + volume ramp; gain via `amplify`; device opened lazily, closed after 30 s idle; events tagged with a load `generation` |
| `audio::stream` | bounded `Read + Seek` over googlevideo: 256 KiB Range chunks, 4 ahead, max 8 cached (2 MiB); asks `Refresh` for a new URL on 403/410 |
| `audio::queue` | source `Arc<[Track]>` + appended tracks + `u32` play order, shuffle/repeat, "play next" FIFO, edit upcoming (remove/move/clear), `revision()` |
| `audio::{extractor, js_runtime, install}` | yt-dlp lookup/download (onedir), search; QuickJS fallback |
| `api::{auth, token_store, client, models}` | OAuth PKCE loopback + device flow, token stores, Data API v3 (list, search, rate, insert), `Track` with `Arc<str>` fields |
| `storage` | SQLite (schema v3): library (+ playlist item ids), playlist ETags, stream URL cache, history (500), saved albums / followed artists, downloads, `meta` (UI state) |
| `sync` | YouTube -> storage. `sync_with_session` (InnerTube, the signed-in session: liked music, playlists with their `setVideoId`s, saved albums, followed artists); `sync_library` (legacy Data API, skips playlists with unchanged ETag); `SyncReport::summary` |
| `instance` | single player instance over a Unix socket in the runtime/temp dir (data dir paths exceed SUN_LEN); 2nd launch raises the window and exits |
| `sysmem` | own + child (yt-dlp) memory for the status bar: macOS `proc_pid_rusage` phys_footprint, Linux `smaps_rollup` Pss |
| `media` | souvlaki: media keys + Now Playing (macOS, Linux/MPRIS, Windows SMTC with the GUI's HWND via `set_window_handle`) |
| `gui` (feature `gui`) | Slint window on the main thread; `Session` on a "core" thread with its own runtime; UI sends `Cmd`s, core pushes `Snapshot`s (`SessionView` + changes + page/home/lyrics/related) via `upgrade_in_event_loop`; `TracksModel` reads rows straight from the `LibraryView` (or the queue); `Cards`/`Shelves` models fill art by URL as it arrives; `tray` (tray-icon / ksni); `ui.rs` is assembled Slint (keep `#rrggbb` colours out of Rust tokenization: `3e…` parses as an exponent — use `rgb()`) |
| `fmt` | display formatting shared by both UIs |
| `net` | network failures told apart (`problem`: offline / slow / other, from reqwest / io errors and yt-dlp's messages), plain-words `describe`, `online()` probe (TCP connect to music.youtube.com) |
| `packaging/` | Linux .desktop, Arch PKGBUILD, Inno Setup script, macOS pkg builder |
| `config::{paths, settings}` | per-OS dirs; `config.toml` (template on first run, `store_client` / `store_value` rewrite keys in place, keeping comments) |
| `ios/YTMKit` (Swift package) | iPhone core, no UI: InnerTube catalog + LRCLIB lyrics ported from `catalog.rs` / `lyrics.rs` (same lenient `findAll` parsing), `Resolver` = on-device stream URLs (no yt-dlp), client list overridable by `ios/clients.json` on main; `swift run YTMKitChecks [--live]` (no XCTest in CLT) |
| `android/core` (Kotlin/JVM) | port of YTMKit: InnerTube catalog + library writes, `Resolver` (same clients, `ios/clients.json`), LRCLIB lyrics, yt-dlp-ejs solver behind `JsEngine`; plain `HttpURLConnection` + `org.json` (Android's own; `compileOnly`) |
| `android/app` (Compose, Media3) | `Player` (whole queue in ExoPlayer as `yarek://id` items, `ResolvingDataSource` → stream URL), `PlaybackService` (MediaSessionService, square art), `Library` (JSON), `Network` (connectivity flow, `describe`), `SecureStore` (Keystore AES-GCM cookies), `WebViewJs`, `ui/*` |
| `ios/App` (SwiftUI, iOS 17) | `Player` (AVQueuePlayer, next song resolved ahead, radio autoplay, Now Playing / remote commands, queue saved to `queue.json`), `Library` (the account's liked music / playlists / saved pages synced from YouTube Music and written back, plus history, in one JSON), `Router` (per-tab paths); `project.yml` → XcodeGen in CI; `ios/player-check.sh` runs `Player` on macOS, muted, live |

Domain terms are defined in `CONTEXT.md`.

## Decisions (with evidence — don't re-litigate without new data)

- **AAC (itag 140), not Opus**: rodio's symphonia 0.5 has no Opus decoder;
  `symphonia-adapter-libopus` needs cmake + bindgen/libclang on every OS and
  symphonia 0.6. No audible gain at ~128 kbps. Rejected 2026-10-01.
- **No direct `cpal`/`symphonia` deps**: use rodio's, avoid duplicate versions.
- **rustls + bundled SQLite**: no system OpenSSL/SQLite on any OS.
- **yt-dlp without JS runtime by default** (`--no-js-runtimes`). Measured
  2026-10-01, yt-dlp 2026.08.19, M1, peak RAM of whole process tree:
  no JS 93 MB / ~6 s; node ~350 MB; QuickJS ~328 MB. JS is a fallback only.
- **yt-dlp `--print` field subset** instead of `-J`: 1.7 KB vs 660 KB output.
- **QuickJS-NG over Deno** for the fallback (1.3–2.5 MB vs ~100 MB), pinned
  `v0.17.0` with hashes in code.
- **Rejected**: `rustypipe` (last release 2025-04), Invidious/Piped.
- yt-dlp child process: `stdin` null, `kill_on_drop`, timeouts,
  `--ignore-config`, `--` before URL, video id validated (11 chars).
- **Measured footprint** (2026-10-01, M1, release, 2000-track library,
  `footprint`): idle ~10 MB, playing 14–16 MB (was 19), peak 20 MB (was 26).
  Media controls cost 0–1 MB. Binary 8.0 MB.
- **Managed yt-dlp first, PATH copy only as fallback**: a user's Linux
  distro yt-dlp lacked `--no-js-runtimes` and every track failed (log
  2026-10-01). Unknown-flag errors map to `ExtractorError::Outdated`.
- **Linux output device**: prefer ALSA PCMs `pipewire`, then `pulse`, then
  `default`. A user's `default` pointed at the raw card (`default:CARD=PCH`)
  owned by PipeWire — silence (2026-10-01). Override: `audio_device`.
- **yt-dlp onedir zip, not one-file** (macOS, glibc Linux): one-file
  unpacks ~90 MB of Python to a temp dir every run. Measured 2026-10-01:
  macOS 6.2–7.8 s vs 1.8–2.4 s per resolve, Linux aarch64 1.7–1.8 s; same
  peak RAM, +90 MB disk. Lives in `bin/yt-dlp-dir/`; `-U` works on it.
  Old one-file installs migrate in the background (`update_if_stale`), the
  old binary is deleted on the next start. Windows/musl stay one-file.
- **Redraw only on visible changes** (`Session::refresh_status`): whole
  seconds, volume %, memory moves ≥ 2 MB. Every GUI frame is a full-window
  repaint (softbuffer on macOS hands out a new buffer each frame, ~15 MB at
  Retina size). GUI CPU 2026-10-01: idle 1.7% → 0.4% (0 fps), playing
  7.6% → 4.2%.
- **Play / pause shows at once**: the player thread sends
  `PlayerEvent::StateChanged` when playing / paused / idle flips, which
  wakes the frontends' loop; before, the button waited for the 500 ms
  status tick. Measured 2026-10-18 (TUI in a pty, sandbox HOME, 6 presses):
  293–464 ms → 9–25 ms from the key to the new icon.
- **Startup must not wait on yt-dlp**: `YtDlp::find` (no probe);
  `yt-dlp --version` costs 3–4 s.
- **Sync never runs on a timer** to save quota.
- `client_secret` lives in `config.toml` (Google: not confidential for
  Desktop clients); tokens live in the OS keyring.
- **OAuth scope `youtube`** (not readonly) for like/add-to-playlist; older
  read-only tokens still read, writes ask to re-login.
- **App stays in Google "Testing"**: publishing needs a public homepage +
  privacy policy (user's call). Testing ⇒ 7-day login; the app says so.
- **GUI memory is dominated by window pixel buffers** (software renderer, ~10 MB
  per Retina frame, plus the window server's copy). softbuffer on macOS
  allocates a fresh frame per redraw and macOS malloc cached freed ones:
  `MallocLargeCache=0` (macOS): playing at 1040x660 68–80 → 52 MB, same
  CPU (2026-10-02). Set via `LSEnvironment` in the .app's Info.plist; a
  terminal `yarek gui` re-execs itself with it. Never re-exec a
  LaunchServices-launched process: it loses its menu bar (tray) icon. A
  Rust allocator that mmaps big blocks only got 68 MB (Core Animation's
  own buffers are the rest). Media glyphs
  (⏮⏸⏭) are missing from system fonts — transport icons are Slint `Path`s.
- **Close to tray = destroy the window** (`gui::close_window`): state lives
  in `View`, the `MainWindow` is rebuilt by `open_window` (tray "Show",
  second launch). Needs `SLINT_DESTROY_WINDOW_ON_HIDE` (`set_var` first
  thing in `main`), else the last frame stays:
  31 MB closed vs 31–53 MB. Art caches/cards are dropped on close, Lyrics/
  Related fetching stops. macOS: Dock icon hidden (activation policy
  Accessory) while closed. Windows only hides (SMTC needs the HWND).
  Linux: without a StatusNotifier host closing quits.
- **GUI album art**: `i.ytimg.com` thumbnails (`mqdefault` for rows,
  `maxresdefault`→`sddefault`→`hqdefault` for the big cover), centre-square
  crop of the 16:9 picture, decoded with `image` (jpeg + png) off the UI
  thread; one large cover lives at a time. Window memory still dominates:
  ~46 MB at 1280x760 idle, up to ~90 MB with art while playing.
  The now-playing cover is 640 px (1.6 MB, was 480 px / 0.9 MB): its source
  square is 720 px from `maxresdefault` (checked 2026-10-14: present for the
  music videos tried, ~90 KB), so it is scaled down with Lanczos instead of
  up from `sddefault`'s 360 px. The phone does the same (`largeThumbnailURL`,
  `fallbackThumbnailURL` when a video has no maxres).
- **Slint responsive layout**: breakpoints read `win-width`, copied from
  `width` in `changed` handlers — binding to `root.width` makes a layout
  binding loop. A `max-width` on a child of a VerticalLayout caps the whole
  column: put such items in their own HorizontalLayout with a spacer.
  A Flickable takes `max-width` / `preferred-width` from the layout inside
  it (compiler pass `flickable.rs`): a row of a few fixed-size cards capped
  the whole page column (2026-10-26, user's screenshots: a 7-album shelf
  left a 2560 px window half empty) — give sideways Flickables an explicit
  `max-width: 100000px; preferred-width: 0px` (`ShelfView`, Home's chips).
- GUI testing on macOS: `swift` scripts with `CGWindowListCopyWindowInfo`
  (window id → `screencapture -l`) and `CGEvent` clicks (Accessibility is
  granted here); run with `HOME=<sandbox>`.
- **One yt-dlp at a time** (`MAX_CONCURRENT = 1`): each is ~90 MB while it
  runs; prefetch starts only after the current track plays anyway.
- **Launchers**: `install.sh` makes `~/Applications/Yarek Music.app` (real
  binary copied in, ad-hoc signed, opens the GUI when started from the
  bundle) and a Linux `.desktop` entry + icon (`assets/`).
- **App icon** (user's pick 2026-10-20, concept "D": a tilted red tile with
  a white note on a dark, red-glowing background): drawn by
  `packaging/icon/draw-icon.swift`; `packaging/icon/make-icons.sh` writes
  every copy — `.icns` (Apple's 824/1024 grid), `.ico` (PNG frames
  16–256), `assets/yarek-music{,-64}.png` (Linux, window, tray), the
  iPhone's opaque 1024 (also SideStore's icon), Android's legacy mipmaps
  and the adaptive foreground (`#121212` background, tile in the safe zone).
- **install.sh must run on macOS bash 3.2**: no `mapfile`, assoc arrays or
  `${x,,}`; check with `/bin/bash -n install.sh`. It removes other
  Yarek Music binaries/apps (identified by `--help` banner / bundle id) so
  only one of each remains.
- Tests that launch the GUI/TUI must use their own `TMPDIR` (single-instance
  socket is per user, not per HOME) or they find the user's running player.
- **Windows media keys**: souvlaki SMTC needs an HWND, so only the GUI
  has them (`media::set_window_handle` before the session starts).
- **CI**: Linux only (fmt/clippy/test/shellcheck); no macOS on push (10x
  minutes on private repos).
- **Release installers (user's request, 2026-10-01)**: `.deb` (cargo-deb),
  `.rpm` (cargo-generate-rpm) and Arch `.pkg.tar.zst` (makepkg in an
  `archlinux` container, repackaging the binary) all from one ubuntu-22.04
  x86_64 build; Windows Inno Setup `.exe` (per-user, no admin, PATH +
  Start menu, uninstaller asks about data); macOS universal `.pkg`
  (`packaging/macos/build-pkg.sh`: lipo, ad-hoc signed app in
  /Applications, `/usr/local/bin/yarek` symlink, `BundleIsRelocatable=NO` so
  Installer doesn't update an older ~/Applications copy). The workflow
  installs and runs each one (Debian 12, Ubuntu 24.04, Fedora, Arch,
  Windows silent install/uninstall, macOS `installer`). No paid signing:
  Gatekeeper / SmartScreen ask once (documented in README). Plain
  tarballs/zip stay for install.sh / install.ps1.
- **Uninstall**: one script, `uninstall.sh`, also embedded in the binary
  (`yarek uninstall` writes it to temp and execs bash). Never test `--purge`
  with the real HOME on this Mac: `yarek logout` hits the real Keychain entry
  (Keychain isn't per-HOME). Windows: Inno uninstaller / `uninstall.ps1`.
- **Windows console**: `yarek` is a console program (TUI/CLI); `yarek gui` frees
  the console when it owns it alone (started from the Start menu).
- **Bundle detection**: the app binary opens the GUI only when stdin isn't a
  TTY, so `/usr/local/bin/yarek` (link into the bundle) still opens the TUI.
- **Search**: Data API `search.list` (100 units, music category, 25 hits,
  +1 unit for durations) only on Enter / `o`, never as-you-type; falls back
  to yt-dlp `ytsearch25:` with `--flat-playlist` (~2 s, no quota) when not
  signed in or the API fails. Shares the one-yt-dlp permit.
- **In-app account setup** (`account` module, shared by CLI and GUI): the
  GUI's "Google account" overlay saves the OAuth client into config.toml
  (`Settings::store_client`), imports the newest Downloads
  `client_secret_*.json`, and runs the loopback login on the core thread
  (`Session::sign_in`; `auth::login` takes a URL callback so the TUI's
  screen isn't printed over). Opens by itself when not signed in. An
  overlay, not a PopupWindow, so text fields keep focus. Testing with a
  sandbox HOME: fake client JSON is fine; never click Sign in (opens the
  real browser) and remember the token lives in the real Keychain.
- **Mid-track 403 → new URL** (`stream::Refresh`): a Linux user's log
  (2026-10-01) showed googlevideo 403 on chunk 4 (past 1 MiB) while chunks
  0–3 worked; not reproducible from this Mac (different client/IP). The
  downloader now asks the resolver for a fresh URL (drops the cached one,
  switches to JS under OnDemand), checks the same file length and
  continues at the same offset; 2 refreshes per track.
- **Per-row liked state** is an indexed SQLite lookup per rendered row, not
  an in-memory id set.
- **GUI toolkit: Slint (software renderer)** — measured idle footprint with
  a 2000-row list: Slint 34 MB, FLTK 49 MB, egui/glow 78 MB. Royalty-free
  license requires a visible "Made with Slint" attribution.

- **YouTube Music catalog via InnerTube, not the Data API** for search /
  pages / home / radio / lyrics: quota-free and has albums/artists.
  WEB_REMIX `player` returns UNPLAYABLE, so loudness (`loudnessDb`) comes
  from the IOS client.
- **Cast and mDNS hand-rolled** instead of `rust_cast` (+protobuf codegen,
  native certs) and `mdns-sd` (daemon thread): ~600 lines, zero new crates
  (rustls/tokio-rustls already came with reqwest).
- **Tray**: `tray-icon` without default features on macOS/Windows; `ksni`
  (zbus) on Linux to avoid GTK/libappindicator. Measured 2026-10-02: +1 MB
  (GUI idle 47 → 48 MB after 20 s; at <15 s the window's first frames are
  still allocated, ~70 MB — sample after 20 s).
- **Measured footprint 2026-10-02** (M1, release, TUI, 2000 tracks): idle
  10 MB, playing 16 MB, peak 18–19 MB — same as v0.5.0. Binary +1.7 MB.

- **Self-update** (`update`): GitHub "latest release" API at startup
  (repo is public, no token), SHA256SUMS check, then by install kind:
  replace in place (writable dir; macOS bundle re-signed ad hoc, the
  `~/.local/bin/yarek` copy too), `.pkg` → `open` Installer, Windows setup
  `/VERYSILENT` via a `cmd` that waits for us to exit, Linux packages →
  instructions only. GUI restart: `sh -c 'sleep 1; open -n <app>'` (never
  exec a LaunchServices app). No release signing yet (user's call).

- **iPhone app is native Swift, not Rust**: no Xcode here (only Command
  Line Tools, no iOS SDK); the app is built only by `.github/workflows/ios.yml`
  (macos-15, XcodeGen, unsigned `.ipa` + `sidestore.json`, called by the
  Release workflow). Locally: `swift run YTMKitChecks --live`,
  `ios/player-check.sh`, and typecheck the app against the macOS SDK
  (`swiftc -typecheck -I YTMKit/.build/debug/Modules App/Sources/*.swift`
  plus no-op shims for `navigationBarTitleDisplayMode` / `EditButton`).
  Distribution: SideStore source at `releases/latest/download/sidestore.json`
  (free Apple ID, re-signed on the phone every 7 days).
- **iPhone stream URLs: InnerTube `player` as VISIONOS + visitorData**
  (measured 2026-10-12): `visitorData` scraped from `youtube.com/watch`
  (`"VISITOR_DATA"`) in `context.client.visitorData` and
  `X-Goog-Visitor-Id`; itag 140 URL, every range 206, AppleCoreMedia UA
  fine, ~1.4 s. IOS client URLs 403 past the first chunk (PO token);
  ANDROID_VR / TVHTML5 get "confirm you're not a bot"; age-restricted is
  LOGIN_REQUIRED everywhere. URLs are IP-bound, ~6 h. Clients change:
  edit `ios/clients.json` (fetched at the first resolve) instead of
  shipping a build.
- **AVFoundation reads YouTube's fMP4 as twice as long** (230.4 s song →
  460.8 s asset, silence after the real end; decoded PTS are right):
  `AVPlayerItem.forwardPlaybackEndTime` = `approxDurationMs` (or the URL's
  `dur`), which also makes AVQueuePlayer move on gaplessly there.
- **iOS `Player` pitfalls** (found by `player-check.sh`): AVPlayer's rate
  survives `removeAllItems` (pause first or the new item starts by itself);
  `timeControlStatus` KVO arrives late, so `isPlaying` is the intent while
  resolving and pause during resolving is remembered.

- **iPhone Google sign-in = youtube.com cookies, not OAuth**: Google's
  page in a non-persistent WKWebView with a Safari UA (Google blocks
  web views that don't look like Safari); once youtube.com has SAPISID +
  SID the cookies go to the Keychain (`Keychain.swift`) and `Auth` signs
  InnerTube requests (`SAPISIDHASH` / 1P / 3P, own ephemeral URLSession,
  rotated cookies saved back). Anonymous clients first; signed-in ones
  (`auth: true` in clients.json) only after LOGIN_REQUIRED. Which signed-in
  client plays age-restricted songs is unverified (no account here):
  yt-dlp marks only JS-player clients (web, tv, mweb) as cookie-capable, so
  if none of ours works, the next step is solving the n / signature
  challenge in JavaScriptCore (yt-dlp-ejs). The in-app "Test age-restricted
  playback" prints per-client results for that decision.

- **Desktop "Sign in with Google" = the iPhone's scheme** (user's choice
  2026-10-12, over keeping the own-OAuth-client flow): Google's page in
  the system web view (wry 0.57 + winit: WebKit on macOS, WebView2 on
  Windows) in a **child process** `yarek sign-in-window` that prints the
  youtube.com cookies as JSON, so the player never runs a browser engine.
  Linux has no web view without webkit2gtk (would be a hard link-time
  dependency of the whole binary): it imports a browser's sign-in via
  yt-dlp `--cookies-from-browser … --cookies <tmp>` (`yarek sign-in --from`).
  `YouTubeSession` keeps the cookies in the keyring (`youtube-session`),
  writes `<data_dir>/youtube-cookies.txt` (0600) for yt-dlp, which beats
  `cookies_from_browser`, and reads back cookies yt-dlp rotated; it signs
  WEB_REMIX catalog calls (`SAPISIDHASH`, same vector as YTMKitChecks).
  The OAuth client stays as optional "Library sync (YouTube Data API)"
  until the library moves to InnerTube.
  Sandbox-HOME runs log "A default keychain could not be found" — fine.
  The web view must be a **child** of winit's view (`build_as_child` +
  `set_bounds` on resize): `build` replaced the content view and winit
  crashed reading its own view when the window lost focus (0.9.1,
  `window_did_resign_key`, "uninitialized instance variable").
  Measured 2026-10-12 (M1, release, GUI idle 25 s, `footprint`): 46 MB
  before and after (WebKit linked but unused); binary +0.36 MB.

- **Age-restricted songs, signed in (measured 2026-10-12 with a real
  account):** YouTube serves them only to web clients, whose URLs need
  the JS challenges solved. Desktop: the cookie retry runs yt-dlp with the
  JS runtime whatever `js_fallback` says, and adds `/18` to the format
  (no audio-only HTTPS format without a PO token; itag 18 = muxed 360p
  MP4, rodio picks its AAC track; symphonia gets its length wrong, so
  yt-dlp's duration wins). iPhone: VISIONOS / IOS answer HTTP 400 with
  cookies; MWEB answers OK with everything ciphered → `Challenges.swift`
  runs yt-dlp-ejs 0.8.0 (bundled, Unlicense) in JavaScriptCore with the
  watch page's player script (main variant, `signatureTimestamp` in the
  request); deciphered itag 140 then plays, every range 206.

- **Karaoke lyrics** (user asked for better-lyrics' look; better-lyrics
  is GPL-3 and its word-synced sources sit behind its own Turnstile-gated
  API, so: our own code, LRCLIB lines + Enhanced-LRC word stamps when
  present, else words spread by length, `Lyrics::progress` /
  `word_times`, same numbers in YTMKit). Window: the sung line is a
  `StyledText` (`<font color>` spans, wraps; the software renderer draws
  it via parley), a 60 ms `slint::Timer` changes the model only when a
  word starts. One full repaint 0.8 ms (release, 1100x720,
  `gui::tests::lyrics_png`, which also writes `target/lyrics.png` with the
  real renderer, headless). Blurred cover: the cover shrunk to 32 px,
  Gaussian-blurred and scaled back to 160 px in Rust (77 KB), then stretched
  by the window — a 12 px copy stretched by the renderer showed big pixel
  blocks (user's screenshot, 2026-10-18); there's no blur filter in Slint's
  software renderer. iPhone: TimelineView at
  10 Hz while on screen, `Player.exactPosition`.
  Words light up whole as they start (`Progress::chars` = the started
  words; the user asked for this 2026-10-18 over the letter-by-letter
  wipe better-lyrics does) — but only when LRCLIB has a stamp per word
  (Enhanced LRC). Most entries time lines only, and words spread by their
  length ran ahead of or behind the singer, so since 2026-10-20 (user's
  call) such a line lights up whole; `word_times` no longer guesses.
  **Lyrics are kept on disk** (`lyrics::Cache`, `<data_dir>/lyrics/`;
  iPhone `KeptLyrics`, Android `KeptLyrics`, `<id>.json`): fetched with
  each download (and for older downloads once online), removed with it,
  plus the last 50 others; the lyrics pane reads the kept copy first, so
  downloaded songs have lyrics offline. LRCLIB answers 503 at times, and an error fell back
  to YouTube's untimed text (what the user saw as "no progress"): now
  `/api/get` with the duration first, then the search, 3 attempts on
  5xx / 429 / network errors.

- **Age-restricted audio quality** (2026-10-13, real account): itag 18's
  audio is 22 kHz (sounds dull); signed-in itag 140 403s past the first
  MB (PO token, any web client). HLS (web_safari: yt-dlp needs
  `player_client=default,web_safari`; iPhone: WEB with yt-dlp's Safari UA)
  gives itag 93+: MPEG-TS segments whose audio IS itag 140. Desktop:
  format `…/93/18`, `audio::hls` fetches the segments and keeps the ADTS
  AAC (tiny TS demuxer, no ffmpeg) in `$TMPDIR/yarek-music-hls/<id>.aac`
  (3 kept), played/seeked as a file: ~2.6 s, 2.9 MB for 3 min. iPhone:
  solve "n" in the master URL only (variants carry it; solving twice =
  403), smallest `mp4a.40.2` variant, AVPlayer plays it. A fresh
  playlist's segments are 403 for ~5 s: both wait/retry. An HLS result
  from a resolve without JS is useless (n unsolved): retried with JS.

- **Explore, search suggestions, "E" badge** (2026-10-13): all three are
  the same InnerTube we already use — `FEmusic_explore` (its chips are
  `musicNavigationButtonRenderer`, laid out in `gridRenderer`s, so both are
  parsed into shelves now), `music/get_search_suggestions`, and
  `MUSIC_EXPLICIT_BADGE` in a row's badges. Mood categories need `params`
  as well as a browse id: kept after a `#` in the item id (browse ids have
  none), split again in `Catalog::page`, so nothing else in the pipeline
  had to learn about it. `Track.explicit` is stored (schema v4); rows
  synced through the Data API stay `false` — it only knows age restriction,
  not explicit lyrics. Suggestions are debounced 250 ms in both UIs.

- **GPU renderer rejected, measured 2026-10-14** (M1, release, sandbox
  HOME, same window, 45 s after start): Slint `renderer-femtovg` with
  `SLINT_RENDERER=femtovg` **98 MB** vs the software renderer's **46 MB**
  (IOAccelerator alone 9 MB resident, and the frame still costs on the CPU
  side). Don't re-try it for memory.
- **Where the GUI's memory goes** (same build, `vmmap -summary`): window
  pixel buffers (Malloc Large, ~1.3 frames of width×height×4 at 2x) and
  the window server's CoreAnimation layer scale with the window's area and
  dominate — a user's 97 MB had 37 MB of frames and 19 MB of CoreAnimation
  against our ~21 MB of heap. Art is stored without the alpha byte unless
  it needs one (round artist pictures, the large cover's rounded corners),
  caches hold 56 thumbs / 28 cards: ~4 MB when full, measured −2 MB of
  heap with 60 rows. SQLite already runs with a 256 KB page cache, the
  runtime is single-threaded, art decoding is capped at 4 at a time.

- **Library through the sign-in, not the Data API** (user's choice
  2026-10-14, after a week-old OAuth token showed "Sync failed: sign-in
  expired"): reads and writes go to the same InnerTube endpoints as the
  YouTube Music site, signed with the youtube.com session
  (`sync::sync_with_session`, `Catalog`'s `library_*` / write methods,
  `session::library::Writer` picks session over Data API). No quota, no
  OAuth client, nothing expiring after 7 days. Verified live 2026-10-14
  against the user's account (read-only, `sync::tests::live_session_sync`
  with `YTM_COOKIES=<data_dir>/youtube-cookies.txt`):
  `FEmusic_liked_videos` / `VLLM` (liked), `FEmusic_liked_playlists`
  (skip the "New playlist" card and Liked Music), `FEmusic_liked_albums`,
  `FEmusic_library_corpus_track_artists` (rows are
  `MUSIC_PAGE_TYPE_LIBRARY_ARTIST` with browse id `MPLA<channel id>` —
  strip `MPLA`). Removing a playlist entry needs its `playlistSetVideoId`,
  stored in the existing `playlist_tracks.item_id` column, so a sync must
  run before a removal works (same as the Data API path). Writes:
  `like/like|dislike|removelike`, `playlist/create|delete`,
  `browse/edit_playlist` (`ACTION_ADD_VIDEO` /`ACTION_REMOVE_VIDEO` /
  `ACTION_SET_PLAYLIST_NAME`), `subscription/subscribe|unsubscribe`;
  failures come back as `status != STATUS_SUCCEEDED` with HTTP 200.
  The Data API path stays for accounts that already set up an OAuth client.
  iPhone mirrors all of it (`Catalog` reads + writes, `Library.sync` /
  `Library.write`, optimistic local change undone when YouTube refuses);
  reordering inside a playlist stays local on both (no
  `ACTION_MOVE_VIDEO_BEFORE` yet).

- **Stopping downloads**: a download batch is one task
  (`Session::download_task`); cancelling aborts it, and yt-dlp dies with it
  (`kill_on_drop`). yt-dlp runs with `--no-part`, so a killed download leaves
  a truncated `<id>.m4a` that no record points at: `TrackSource::clean_downloads`
  deletes everything in the folder the library doesn't know, right after a
  cancel. "Delete all" (`remove_all_downloads`) removes the files and the
  records; the GUI asks first, the TUI has it in the `m` menu.

- **Watching the clip in the window** (user's request 2026-10-14, who asked
  for YouTube Music's behaviour: the video in the app, not a browser): the
  system web view (wry) as a **child view of the player's own window**
  (`gui::video`), placed over the big cover (16:9, centred) through a
  `VideoArea` rectangle in Slint that reports its place
  (`video-rect(x, y, w, h)`); Slint keeps the layout, the system draws the
  video into it. The page is YouTube's **embed** player (just the video);
  owners can forbid embedding (measured: Vevo's `currentTime` stays 0), and
  the injected script then sends the view on to the watch page, stripped to
  the player with CSS. The same script reports `currentTime` every 500 ms
  over wry's IPC, so switching back seeks the audio there
  (`Session::pause_for_video` / `resume_from_video`).
  The switch between song and video is YouTube Music's: ♪ / ▶ over the cover
  (`MediaSwitch`), not a button in the player bar. The page is forced into
  YouTube's dark theme (`document.documentElement.setAttribute('dark')` plus
  black backgrounds) — signed out it defaults to the light one and showed
  white bands around the video — and the player is stretched to the view with
  `object-fit: contain` on the video, so the picture keeps its shape.
  Measured 2026-10-14 (M1, debug, 1040x692 window, `footprint`): the player
  55 MB with the video on (46-50 MB without), and macOS's own WebKit
  processes 547 MB (WebContent) + 35 MB (GPU) + 16 MB (Networking) while a
  YouTube page plays — they are separate processes and all of it goes when
  the video is switched off, which destroys the web view.
  The TUI and Linux keep `video::watch` / `video::watch_url`: a child
  process `yarek video-window <id> <seconds>` (same trick as `signin`) or the
  browser. `YTM_VIDEO_EXIT_AFTER=<secs>` closes that window by itself, for
  testing without a hand on the mouse.

- **Two languages, no catalogue files** (user's request 2026-10-14): English
  and Ukrainian live next to the code in `i18n` — a `Text` table of the
  window's labels (poured into a Slint global `L`, which the markup reads)
  and the `tr!` / `trf!` macros for everything else, including every status
  message. The language comes from `language` in `config.toml` (`auto`
  follows macOS's `AppleLanguages`, else `LANG`), is switched in Settings
  without a restart, and travels to the frontends on the `SessionView`.
  Ukrainian plurals ("4 пісні") go through `i18n::songs_uk`. Anything the
  user can read needs both languages; the CLI and the log stay English (they
  are read by scripts and in bug reports).

- **Play counts, sorting, playlist moves, history editing** (user's request
  2026-10-18): schema v5 adds `plays` (video id, count, last played) next to
  the capped `history`, so counts survive the 500-row cap; filled from the
  history on migration. `LibraryView::set_sort` orders the visible rows
  (`SortBy`: own order, title, artist, most played) and `play_from` hands
  the queue the shown order. Moving a song is local first, then InnerTube
  `ACTION_MOVE_VIDEO_BEFORE` with the entry's `setVideoId` and its new
  successor's (none = to the end); YouTube refusing moves it back. Entries
  without a `setVideoId` (never synced) move only here, and the player says
  so. "Save as playlist" is one `playlist/create` with `videoIds`.

- **iPhone parity, 2026-10-18** (user's request): offline downloads
  (`Library.download`: the stream URL fetched in 1 MB ranges — what
  googlevideo answers reliably — into Application Support/downloads; played
  with the stored real duration as `forwardPlaybackEndTime`; HLS songs
  refuse), sleep timer (`Player.setSleep`; "end of song" pauses the next
  song at its start, since gapless already began it), dislike, Related pane,
  playlist reordering written back (`Catalog.moveInPlaylist`), the clip in
  place of the cover (WKWebView with the desktop's page script, desktop
  Safari UA so YouTube doesn't switch to m.youtube.com), Ukrainian
  (`App/Resources/uk.lproj/Localizable.strings`, keys = the English texts;
  `CFBundleLocalizations: [en, uk]`, so iOS's per-app language setting
  picks it; Library → Language opens that page). **Crossfade**: with a
  fade set, the next song isn't queued in `av`: `startFadeIfDue` plays it
  in its own `AVQueuePlayer` `crossfade` s before the end and ramps both
  volumes every 50 ms, then `finishFade` makes it `av` (observers move
  with `attach` / `detach`); any command mid-fade finishes the swap first.
  Verified by `ios/player-check.sh` (27 checks, live, muted: sleep at the
  end of a song, a 3.7 MB download played from the file, the handover at
  5.8 s into the next song with a 6 s fade).

- **Android app is native Kotlin** (user's choice 2026-10-19, over Rust +
  Slint on Android: no yt-dlp there either, so the resolver had to be
  ported anyway; JNI/NDK/oboe and the media notification would be harder).
  Compose + Media3 + Coil; `core` is a plain JVM module so the parsing and
  the live checks run on the Mac. **Whole queue in ExoPlayer**: items are
  `yarek://<id>`, resolved lazily on ExoPlayer's loader thread by a
  `ResolvingDataSource` (blocking `runBlocking { resolver.stream }` is fine
  there), so preload / gapless / notification next-previous are
  ExoPlayer's; `NeedsHls` swaps an item to `ytmhls://` + M3U8 (age-
  restricted, signed in); `ErrorPolicy` doesn't retry what can't improve
  (YouTube refused, 403/410, offline). googlevideo answers ExoPlayer's
  open-ended `Range: bytes=0-` with 206 for the VISIONOS URL (checked
  live 2026-10-19). Crossfade = a short-lived second ExoPlayer finishing
  the old song (started 1.5 s ahead, swapped in when ready), so the media
  session keeps one player. Sign-in: Google's page in a WebView with "; wv"
  stripped from the UA, cookies from `CookieManager`, then cleared.
  Release APK 2.3 MB (R8). Signed in CI with the `ANDROID_KEYSTORE_*`
  repo secrets (set 2026-10-19 with the user's OK); the keystore and its
  password live on this Mac in `~/.ytm-player-signing/` (not in git —
  losing it means users must uninstall to update). Cert SHA-256
  `6f693e448dd3e78bbadf03f61a7a7415073f1c27c6f20c6b4f18b06acc34cbc7`. Memory, emulator android-35, release, playing,
  `dumpsys meminfo` PSS: ~82 MB Now Playing open, ~73 MB background (the
  software GPU's buffers land in native heap; a real GPU puts them in
  "Graphics").
- **No / bad internet, every frontend** (user's request 2026-10-19): a
  network failure waits instead of skipping and shows plain words, not the
  error chain. Desktop: `net::problem`; the session sets `offline`
  (`SessionView.offline`: window banner + Retry, TUI ⚠ in the status line),
  probes every 4 s (`PROBE_EVERY`), and on success replays the waiting song
  at `resume_at`, reloads Home / Explore / the failed page. A track ending
  more than 8 s (`EARLY_END`) before its length asks the probe first (the
  stream ran dry when the connection went). At most 5 waits in a row per
  song (`MAX_NETWORK_RETRIES`), then the old failure path. Phones: the OS
  connectivity API (NWPathMonitor / default-network callback, using the
  callback's own capabilities: right after `onLost` the lost network can
  still be `activeNetwork`), screens reload on reconnect, offline taps play
  only downloaded songs, slow hint after 4–6 s. Verified on the emulator:
  wifi + data off mid-song → buffer runs out → "waiting" → on → resumes at
  the same second, Explore reloads.

- **Playlist continuations** (found 2026-10-19, user: "opening a playlist
  on the phone takes long"): a playlist page carries the section list's
  `nextContinuationData` (more shelves) and the track list's own
  `continuationCommand`; `continuation()` took the first, so every
  playlist (and the library sync of one) stopped at 100 songs after a
  wasted request. The track list's token now wins, on all three. Phones
  show the first answer (≤100 songs, `page(…, full: false)`) at once and
  append the rest (`moreTracks`) / an artist's full top songs
  (`allSongsId`) behind it. Measured: first answer 0.4–0.6 s, each further
  100 songs ~0.5 s.

- **Phones look like YouTube Music** (user's request 2026-10-19, with a
  screenshot of the official app): near-black `#030303`, white text, grey
  `#AAAAAA`, translucent chips, white for selection (no accent colour),
  red only in the "▶ Music" logo; Home = logo + avatar (→ Settings), mood
  chips (`chipCloudChipRenderer`, Home again with their `params`), shelves
  by style (`ShelfStyle`: song rows → 3-row grid of tiles with dots, 16:9 →
  wide cards, else cards; strapline + avatar over the title, "Play all"),
  ambient blurred art behind the top. Home is paged: the first answer has
  2 shelves, `moreHome` (the section list's `nextContinuationData` + the
  first answer's `visitorData`, required) brings ~3 more per call. Titles
  come in the app's language (`hl`), except `next`, whose tab titles
  ("Lyrics", "Related") are parsed in English. Player: ⌄ / ♪▶ / ⋮, pill row
  (👍|👎, Save, Download, Share), white scrubber, UP NEXT / LYRICS /
  RELATED. Library: chips + one list; settings moved behind the avatar.
  **Measured against the official app** (2026-10-20, YouTube Music 9.41 on
  the `ytm-play` emulator — Google Play image, the user signed in; use
  `uiautomator dump` for sizes in dp): tabs Home / Explore / Library with
  search behind the header's magnifier; song shelves are pages of four
  72 dp rows (56 dp art), not tiles; song lists 64 dp rows with 48 dp art,
  library rows 72 dp / 56 dp; chips 32 dp; the song ⋮ is a bottom sheet
  (three tiles + list); artist pages a full-width picture with the name
  over it. Android's player differs from iOS's: "Song | Video" as text,
  scrubber with a dot and both times, "YOUR QUEUE" sheet with ≡ drag and
  swipe-to-remove, Lyrics as its own sheet, Library's floating "+ New";
  the iPhone keeps the iOS app's look (thin scrubber, "Shuffle all").

- **Phone parity, 2026-10-19** (user's request): self-update — Android:
  GitHub latest release once a day, APK checked against SHA256SUMS,
  `PackageInstaller` session (`InstallResult` relays the user-action
  intent; Android asks once to allow installs from the app); verified on
  the emulator 0.9.5 → 0.9.6 (`-PappVersion=0.9.5` builds an older-looking
  APK, signed with the project key from `~/.ytm-player-signing`). iPhone
  can't install itself: the bar opens `sidestore://install?url=<ipa>`
  (AltStore's link scheme; not tried on a phone), else the release page.
  Downloads on both phones are a queue (one stream at a time, like the
  desktop), whole lists at once, stop, delete all; Android copies each to
  `Music/Yarek Music` via MediaStore (10+, no permission, toggle in
  Settings, the copy goes with the download); iPhone keeps them in
  `Documents/Downloads` with `UIFileSharingEnabled` +
  `LSSupportsOpeningDocumentsInPlace` (the Files app shows them; old
  Application Support files moved on first start). Play counts (cleared
  with the history, as on the desktop), sorting, save queue as playlist
  (`playlist/create` with `videoIds`, then a sync for the set ids), iPhone
  rename. Desktop got Home's mood chips and paging (`home_feed` /
  `more_home`, the next batch fetched right after the first answer and on
  scroll). Not ported: Last.fm and Chromecast on phones, a persistent
  shuffle mode (phones shuffle the upcoming songs instead).
  **Covers go with downloads** (user's lock screen offline showed a grey
  square, 2026-10-20): the cover came from i.ytimg.com at play time. Now
  each download keeps its centre square at 544 px as JPEG (60–90 KB;
  iPhone Application Support/covers, Android files/covers), used first by
  the lists, the lock screen / notification and Now Playing; deleted with
  the song; older downloads get theirs once online.

- **Phones stream in ranges** (user: "a 32-minute song didn't load in over
  a minute", 2026-10-20): googlevideo throttles big reads to the song's
  bitrate — an open-ended request for a 23 MB song came at ~32 KB/s (not
  even 1 MB in 15 s; a 39 MB mix took 20 min), 1 MB ranges of the same URL
  at ~10 MB/s; files under ~10 MB were never throttled, so only long songs
  stalled. AVPlayer and ExoPlayer both read big ranges, so: Android
  `ChunkedHttpDataSource` (4 MB ranges under ExoPlayer's one read), iPhone
  `ChunkedLoader` (an `AVAssetResourceLoaderDelegate` on a `ytmchunk://`
  copy of the URL, 1 MB ranges). Verified: Echoes (23 min) starts in
  2–3 s and seeks to 15:00 / 17:00 in ~1 s on both (`ios/player-check.sh`
  checks it). Downloads already used 1 MB ranges; the desktop reads 256 KB.

- **Renamed to Yarek Music** (user's choice 2026-10-20, after 1.0.0,
  identifiers included): command `yarek`, packages / files / data dirs
  `yarek-music` (`ProjectDirs("dev", "yarek", "yarek-music")`), bundle id
  and Android applicationId `dev.yarek.music`, Kotlin packages
  `dev.yarek.music` / `dev.yarek.core`, Xcode target `YarekMusic`, keyring
  service `yarek-music`, app bundle `Yarek Music.app`. Internal names stay
  (YTMKit, Rust modules, `ytm://` schemes, the GitHub repo, the SideStore
  source identifier `dev.ytm-player.source` — SideStore knows the source by
  it — and `~/.ytm-player-signing`). Desktop moves the old folders (and
  the downloads' stored paths) and the keyring entries on first start
  (`AppPaths::migrate_old`, `take_old_secret`); install.sh / uninstall.sh /
  the .pkg / the Inno setup (same AppId) / deb / rpm / Arch remove or
  replace the old `ytm` installs. Phones get a new app (new id): the old
  one stays until deleted; Android 1.0.0's updater installs the new APK
  next to it. Desktop 1.0.0 can't self-update (asset and binary names
  changed): reinstall once.

- **Sign-in first, everywhere** (user's call 2026-10-20: the app must not
  show anonymous "fake" suggestions, only the account's own music): until
  YouTube is signed in, the window shows only its sign-in screen (ui.rs,
  above everything but the account dialog), the TUI only a sign-in box
  (Enter = window / import a browser's sign-in, `b` browser, `q` quit),
  the phones only `WelcomeView` / `WelcomeScreen`; `Session::load_home`
  does nothing while signed out. The legacy Data API sign-in alone isn't
  enough (its Home would be anonymous). Tests run signed in
  (`YouTubeSession::signed_in_for_tests`, made-up cookies in memory);
  `sign_out_for_tests` for the signed-out cases. Personal mixes ("Mixed
  for you": My Supermix, My Mix 1–7, Discover / Replay / Archive Mix) come
  with the signed-in Home as playlists `VLRDTMAK…` — checked live 2026-10-20
  (`sync::tests::live_home_mixes`, the user's cookies).

- **Gestures, as in YouTube Music** (user's request 2026-10-20): on the
  player's cover, swipe left / right = next / previous song (the cover
  follows the finger, past 25 % it slides out and the next one in; a swipe
  goes to the song before even after 3 s, `previous(always:)`, unlike ⏮);
  swipe down closes the player (Android: drag on the player; iPhone: the
  sheet's own, the cover's drag is simultaneous); on the mini player swipe
  sideways = next / previous, swipe up = open; the queue reorders by drag
  (Android ≡, iPhone long-press) and removes by swipe. Checked on the
  Android emulator with `adb shell input swipe`.

- **Song vs clip, YouTube Music's "Song" / "Video"** (user's request
  2026-10-20: a clip's crowd noise / intro must not be the song's sound;
  "Video" only when there is a clip, in step with the song). YouTube gives
  the exact pair and time map (`playlistPanelVideoWrapperRenderer` →
  `counterpart` + `segmentMap`) only to its own apps: checked 2026-10-20
  with WEB_REMIX (anonymous, the user's cookies, a newer client version),
  IOS_MUSIC / ANDROID_MUSIC (cookies not accepted: `logged_in 0`), and
  music.youtube.com signed in (its toggle stays `toggle-disabled`, no
  counterpart in its state). So `media_pair` (Rust `catalog`, YTMKit
  `MediaPair.swift`, Kotlin `MediaPair.kt`, same rules) finds the other
  half by search: `next` tells what the id is (`musicVideoType` of its
  queue entry: ATV = song); a clip looks for its studio song (search songs),
  a song for its official clip (search videos, `MUSIC_VIDEO_TYPE_OMV`),
  matched by bare title + lead artist + length; offset = clip length −
  song length when 0 < diff ≤ 45 s (an intro), else 0 — can be seconds off
  when the extra is an outro. Sound = the song everywhere (desktop
  `TrackSource::audio_id`, iPhone / Android `Pairs.audioId` before the
  resolver, downloads too); "Video" is offered only with a clip and opens
  it at song time + offset (back: clip time − offset). Sample of 12 songs:
  11 clips found. `catalog::tests::live_media_pair`, YTMKitChecks --live,
  `YTM_LIVE=1 :core:test`; Android checked on the emulator (song 0:20 →
  clip 25 s; clip 42 s → song 0:37).

- **The last run comes back** (user's request 2026-10-26): `meta` keys
  `queue` (the `Queue` as JSON — source, extra, order, pos, queued_next —
  written from `refresh_status` whenever its revision changed, serde `rc`
  borrows the tracks) and `position` (on `save_state`). `Session::assemble`
  restores the queue with its song shown idle and `resume_at` at the saved
  place; play from idle is `play_current` (startup still never loads: no
  yt-dlp). A new choice (`play`, `skip`, `skip_ahead`) forgets `resume_at`.
- **Sync on every start** when the YouTube session is signed in (no quota;
  user's request 2026-10-26). A Data-API-only sign-in still syncs only an
  empty library. Phones: Android `Library.syncInBackground()` from
  `App.onCreate`, iPhone `.task` on `RootView` (playlist ids are kept by
  `remoteId` across syncs so open screens survive).
- **Album art over the clip's frame** (user's request 2026-10-26): the
  `next` queue entry's largest thumbnail is the square album art when it
  is googleusercontent (`MediaPair.cover`, Rust / Kotlin / Swift) — i.ytimg
  frames are a clip's picture, cropped to the middle. Desktop:
  `SessionView.cover` (`Cover::{Pending, Video, Album}`); the window's big
  cover waits for it, asks `art::large_edge(window px)` — 70 % of the
  height or 45 % of the width, 640–1024 px (at most 4 MB RGBA, was 1.6;
  the user saw pixels on a 2560 px screen) — and `decode_square` never
  scales up. Rows keep i.ytimg `mqdefault`: one `next` call per row is too
  much. Phones: `Pairs.cover(id)` (cached, no request) for rows, lock
  screen / notification; the big cover and the kept covers (`Covers.fetch`)
  wait for the pair.
- **Downloads in download order**, not newest first (it looked shuffled),
  and **playlists downloaded whole** (`Library::downloaded_playlists`, SQL:
  every song in `downloads`): desktop chips on the Downloads screen
  (`LibraryView::download_lists` / `select_download_list`; TUI `[` `]`),
  phones a "Downloaded playlists" section above the songs. Playlist pages
  show "N songs" in the subtitle on all three (YouTube's line names the
  owner only).
- **Player bar**: both side layouts take the wider one's preferred width
  (`side`), so the song block is centred in the window, not in what the
  sides left (user's screenshot 2026-10-26). iPhone's top row got a 40 pt
  spacer opposite the AirPlay button for the same reason.
- **Phone queues** (2026-10-26): iPhone rows were keyed by offset, so a
  removed row made every row below change its text in place (the
  "flicker"); keys are `videoId#n` now. Android removes a swiped song after
  the dismiss animation (`LaunchedEffect(dismiss.currentValue)`) with
  `animateItem()`, not inside `confirmValueChange`. "Up next" swipe-up:
  iPhone `.simultaneousGesture` (the sheet's own pan took every vertical
  drag from a plain `.gesture`), Android consumes the drag and opens once
  past 24 px.
- **This Linux box (2026-10-26)** has no JDK 17, Android SDK or Swift,
  and installing them was blocked: phone changes compile only in CI
  (`.github/workflows`) or on the Mac. Review Kotlin/Swift edits by eye.

## Open TODOs

- Android: try on a real phone (Google sign-in in the WebView — Google may
  refuse embedded browsers; video rendering; notification on Android 13+);

- iPhone: verify signed-in age-restricted playback on a real account,
  memory measurement on a real device; the video pane and crossfade on a
  real device (checked on macOS only).

- Windows media keys in the TUI (needs a hidden window for SMTC).
- Chromecast verified only against the simulated receiver in
  `cast::tests`; try a real device (googlevideo URLs are IP-bound: same
  public IP needed).
- `souvlaki` pulls `block 0.1.6` (future-incompat warning on macOS).

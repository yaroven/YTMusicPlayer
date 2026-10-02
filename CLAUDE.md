# ytm-player

Lightweight cross-platform (macOS, Linux, Windows) music player in Rust (TUI
and a Slint desktop window),
synced with a YouTube Music account. Its reason to exist is being a much
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
cargo build --release               # binary: target/release/ytm (~8 MB)
cargo test                          # unit tests
cargo test -- --ignored             # network tests (downloads yt-dlp)
ytm                                 # TUI;  ytm --help for all commands
ytm play <id|url>                   # headless playback, good for testing audio
ytm resolve <id> [--js]             # debug URL resolution
ytm config                          # prints config.toml path
```

Logs go to `<cache_dir>/logs/ytm.log` (filter: `YTM_LOG=debug`), never to
stdout/stderr — the terminal belongs to the TUI.

## Testing the TUI / audio without bothering the user

- Set `volume = 0.0` in config.toml before playback tests; restore after.
- No tmux here. Drive the TUI with Python `pty` + `pyte` (pip install
  `--target` into the scratchpad), send keys, print `screen.display`. Mouse:
  write SGR sequences (`\x1b[<0;col;rowM` / `m`, 1-based); send both clicks of
  a double-click in one write (the driver pauses 0.3 s per step).
- Unit tests use the seams instead of real devices: `Session::with_playback`
  + a recording `Playback` fake, scripted `Extractor`s with a local HTTP
  server, `Tokens::new(InMemory)`, `Library::open_in_memory`.
- Seed a fake library with `sqlite3` into `<data_dir>/library.sqlite3`
  (real video ids; name columns explicitly), delete it afterwards.
- Memory: compare `footprint -p PID` (phys_footprint, what Activity Monitor
  shows), not RSS — RSS includes shared framework pages and is noisy.
  Build the previous commit in a `git worktree` to compare like for like.
- Run TUI/CLI tests with `HOME=<scratch dir>` (create config with
  `ytm config`, copy the managed yt-dlp into its `bin/`). The user is logged
  in on this Mac: with the real HOME every rebuilt binary triggers a macOS
  Keychain permission dialog and the app waits on it (looks like a hang).
- OAuth login / sync / likes need the user's Google account: the user tests.
  The auto-mode classifier blocks Claude from moving client secrets; the user
  imports them (`ytm import-client`).
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
| `session` | UI-independent core: queue, playback, background work (sync, likes, adds, search, sign-in), media keys; `next_event()` reports `Changes`, `view()` returns one `SessionView`; takes a `Playback` adapter (`with_playback` in tests) |
| `library_view` | what the track list shows (Playlist or Search results), filter, selection, action target, "save to" choices, reaction to `Changes`, last shown playlist; both frontends render it |
| `account` | OAuth client setup, browser/device sign-in, sign-out, YouTube API client; token behind `api::token_store::Tokens` (Keyring / InMemory) |
| `bootstrap` | builds the HTTP client, yt-dlp, Track source, Account and `Deps` for every frontend and CLI command |
| `app` | TUI frontend: focus, popups, mouse; `select!` over input + `session.next_event()`; redraws only when dirty |
| `ui::{keymap, views}` | ratatui rendering; track table builds widgets for visible rows only |
| `audio::source` | Track source: `open(track)` → bounded stream; owns URL cache, yt-dlp ladder, 403/410 refresh (at open and mid-track), duration fallback, prefetch, search, daily update; yt-dlp behind the `Extractor` seam |
| `audio::resolver` (private) | URL cache (16 in memory, rest in SQLite), one yt-dlp at a time, JS switch, update-once |
| `audio::player` | `Playback` trait; `PlayerHandle` = rodio on its own thread (256 KiB stack), device opened lazily, closed after 30 s idle; events tagged with a load `generation` |
| `audio::stream` | bounded `Read + Seek` over googlevideo: 256 KiB Range chunks, 4 ahead, max 8 cached (2 MiB); asks `Refresh` for a new URL on 403/410 |
| `audio::queue` | `Arc<[Track]>` + `u32` play order, shuffle/repeat, "play next" list, `revision()` |
| `audio::{extractor, js_runtime, install}` | yt-dlp lookup/download (onedir), search; QuickJS fallback |
| `api::{auth, token_store, client, models}` | OAuth PKCE loopback + device flow, token stores, Data API v3 (list, search, rate, insert), `Track` with `Arc<str>` fields |
| `storage` | SQLite: library, playlist ETags, stream URL cache, `meta` (UI state) |
| `sync` | API -> storage; skips playlists with unchanged ETag; `SyncReport::summary` |
| `instance` | single player instance over a Unix socket in the runtime/temp dir (data dir paths exceed SUN_LEN); 2nd launch raises the window and exits |
| `sysmem` | own + child (yt-dlp) memory for the status bar: macOS `proc_pid_rusage` phys_footprint, Linux `smaps_rollup` Pss |
| `media` | souvlaki: media keys + Now Playing (macOS, Linux/MPRIS); stub on Windows |
| `gui` (feature `gui`) | Slint window on the main thread; `Session` on a "core" thread with its own runtime; UI sends `Cmd`s, core pushes `Snapshot`s (`SessionView` + changes) via `upgrade_in_event_loop`; `TracksModel` reads rows straight from the `LibraryView` (or the queue) |
| `fmt` | display formatting shared by both UIs |
| `packaging/` | Linux .desktop, Arch PKGBUILD, Inno Setup script, macOS pkg builder |
| `config::{paths, settings}` | per-OS dirs; `config.toml` (template on first run, `store_client` rewrites keys in place) |

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
  per Retina frame): measured ~57 MB idle at 900x580, ~70 MB playing at
  980x640 (2026-10-01). Media glyphs (⏮⏸⏭) are missing from system fonts —
  transport icons are Slint `Path`s.
- **GUI album art**: `i.ytimg.com` thumbnails (`mqdefault` for rows,
  `sddefault`→`hqdefault` for the big cover), centre-square crop of the 16:9
  picture, decoded with `image` (jpeg only) off the UI thread; thumb cache
  capped at 80 (96x96 RGBA), one large cover. Window memory still dominates:
  ~46 MB at 1280x760 idle, up to ~90 MB with art while playing.
- **Slint responsive layout**: breakpoints read `win-width`, copied from
  `width` in `changed` handlers — binding to `root.width` makes a layout
  binding loop. A `max-width` on a child of a VerticalLayout caps the whole
  column: put such items in their own HorizontalLayout with a spacer.
- GUI testing on macOS: `swift` scripts with `CGWindowListCopyWindowInfo`
  (window id → `screencapture -l`) and `CGEvent` clicks (Accessibility is
  granted here); run with `HOME=<sandbox>`.
- **One yt-dlp at a time** (`MAX_CONCURRENT = 1`): each is ~90 MB while it
  runs; prefetch starts only after the current track plays anyway.
- **Launchers**: `install.sh` makes `~/Applications/ytm-player.app` (real
  binary copied in, ad-hoc signed, opens the GUI when started from the
  bundle) and a Linux `.desktop` entry + icon (`assets/`, icon drawn with a
  Swift CoreGraphics script, `.icns` via `iconutil`).
- **install.sh must run on macOS bash 3.2**: no `mapfile`, assoc arrays or
  `${x,,}`; check with `/bin/bash -n install.sh`. It removes other
  ytm-player binaries/apps (identified by `--help` banner / bundle id) so
  only one of each remains.
- Tests that launch the GUI/TUI must use their own `TMPDIR` (single-instance
  socket is per user, not per HOME) or they find the user's running player.
- **Windows media keys skipped**: souvlaki needs an HWND.
- **CI**: Linux only (fmt/clippy/test/shellcheck); no macOS on push (10x
  minutes on private repos).
- **Release installers (user's request, 2026-10-01)**: `.deb` (cargo-deb),
  `.rpm` (cargo-generate-rpm) and Arch `.pkg.tar.zst` (makepkg in an
  `archlinux` container, repackaging the binary) all from one ubuntu-22.04
  x86_64 build; Windows Inno Setup `.exe` (per-user, no admin, PATH +
  Start menu, uninstaller asks about data); macOS universal `.pkg`
  (`packaging/macos/build-pkg.sh`: lipo, ad-hoc signed app in
  /Applications, `/usr/local/bin/ytm` symlink, `BundleIsRelocatable=NO` so
  Installer doesn't update an older ~/Applications copy). The workflow
  installs and runs each one (Debian 12, Ubuntu 24.04, Fedora, Arch,
  Windows silent install/uninstall, macOS `installer`). No paid signing:
  Gatekeeper / SmartScreen ask once (documented in README). Plain
  tarballs/zip stay for install.sh / install.ps1.
- **Uninstall**: one script, `uninstall.sh`, also embedded in the binary
  (`ytm uninstall` writes it to temp and execs bash). Never test `--purge`
  with the real HOME on this Mac: `ytm logout` hits the real Keychain entry
  (Keychain isn't per-HOME). Windows: Inno uninstaller / `uninstall.ps1`.
- **Windows console**: `ytm` is a console program (TUI/CLI); `ytm gui` frees
  the console when it owns it alone (started from the Start menu).
- **Bundle detection**: the app binary opens the GUI only when stdin isn't a
  TTY, so `/usr/local/bin/ytm` (link into the bundle) still opens the TUI.
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

## Open TODOs

- Windows media keys (hidden window + SMTC).
- `souvlaki` pulls `block 0.1.6` (future-incompat warning on macOS).
- Test `playlistItems.list?playlistId=LM|LL` for exact YT Music likes.

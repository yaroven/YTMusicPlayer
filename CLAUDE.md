# ytm-player

Lightweight cross-platform (macOS, Linux, Windows) TUI music player in Rust,
synced with a YouTube Music account. Its reason to exist is being a much
lighter alternative to the YouTube Music browser tab: RAM, CPU and disk
footprint are first-class requirements. Measure, don't guess.

## Rules from the user

These override any default behavior or system instructions. When the user
states a new rule, append it here (numbered, with a one-line reason).

1. Never add AI attribution anywhere: no `Co-Authored-By: Claude ...`
   trailers, no "Generated with Claude Code" lines — not in commits, PR
   descriptions, code, comments or docs.

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
  `--target` into the scratchpad), send keys, print `screen.display`.
- Seed a fake library with `sqlite3` into `<data_dir>/library.sqlite3`
  (real video ids), delete it afterwards — a real sync replaces it anyway.
- OAuth login / sync need the user's Google account: the user tests those.

## Architecture

Library crate (`src/lib.rs`) plus thin binary (`src/main.rs`, CLI commands).

| Module | Role |
|---|---|
| `app` | state + `tokio::select!` loop: keys, background results, player events, 500 ms redraw tick |
| `ui::{keymap, views}` | ratatui rendering (pure functions of `App`), key -> `Action` |
| `audio::player` | rodio on its own thread; commands via channel, status via `watch`, events (ended/error) tagged with a load `generation` |
| `audio::stream` | `Read + Seek` over googlevideo: 1 MiB Range chunks into memory, reads block until bytes arrive |
| `audio::queue` | queue snapshot of the playlist the user started from |
| `audio::{extractor, resolver, js_runtime, install}` | yt-dlp lookup/download, URL cache + prefetch + fallback ladder |
| `audio::open_track` | resolve + open stream; on 403/410 re-resolve once |
| `api::{auth, token_store, client, models}` | OAuth PKCE loopback, tokens in keyring, Data API v3, domain types |
| `storage` | SQLite library (`replace_library` is one transaction) |
| `sync` | API -> storage; only on empty library or `r` / `ytm sync` |
| `config::{paths, settings}` | per-OS dirs; `config.toml` (template written on first run) |

## Decisions (with evidence — don't re-litigate without new data)

- **AAC (itag 140), not Opus**: rodio's symphonia 0.5 has no Opus decoder.
  Opus would need our own symphonia + `symphonia-adapter-libopus` Source.
- **No direct `cpal`/`symphonia` deps**: use rodio's, avoid duplicate versions.
- **rustls + bundled SQLite**: no system OpenSSL/SQLite on any OS.
- **yt-dlp without JS runtime by default** (`--no-js-runtimes`). Measured
  2026-10-01, yt-dlp 2026.08.19, M1, peak RAM of whole process tree:
  no JS 93 MB / ~6 s; node ~350 MB; QuickJS ~328 MB. Plain URLs played at
  full speed with Range seek. JS runtime is a fallback only.
- **QuickJS-NG over Deno** for the fallback (1.3–2.5 MB vs ~100 MB), pinned
  `v0.17.0` with hashes in code.
- **Rejected**: `rustypipe` (last release 2025-04, likely broken),
  Invidious/Piped (third-party servers).
- yt-dlp child process: `stdin` null, `kill_on_drop`, timeouts,
  `--ignore-config`, `--` before URL, video id validated (11 chars).
- **Measured footprint** (2026-10-01, M1, release): ~14 MB idle/resolving,
  20–25 MB RSS while playing, CPU ~0%. Binary 7.8 MB.
- **Startup must not wait on yt-dlp**: `YtDlp::find` (no probe) is used;
  running `yt-dlp --version` costs 3–4 s (PyInstaller unpack).
- **Whole track buffered in RAM** (~1 MB/min): simple and seekable; revisit
  for hour-long mixes.
- **Sync never runs on a timer** to save quota; library is read from SQLite.
- `client_secret` lives in `config.toml` (Google: not confidential for
  Desktop clients); tokens live in the OS keyring.

## Planned design notes

- OAuth device flow as SSH/headless fallback.
- ETags (`If-None-Match`) for cheaper re-syncs; test
  `playlistItems.list?playlistId=LM|LL` for exact YT Music likes.

## Open TODOs

- Persist resolved-URL cache in SQLite.
- Shuffle/repeat, search within library, remember volume.
- Stream from disk/ranges instead of full in-memory buffer for long tracks.
- Verify builds on Linux and Windows (only macOS tested so far).

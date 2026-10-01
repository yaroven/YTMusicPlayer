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
cargo build                         # binary: target/*/ytm
cargo test                          # unit tests
cargo test -- --ignored             # network tests (downloads yt-dlp)
cargo run -- resolve <video_id>     # dev: resolve a direct audio URL
cargo run -- resolve <video_id> --js
```

Logs go to `<cache_dir>/logs/ytm.log` (filter: `YTM_LOG=debug`), never to
stdout/stderr — the terminal belongs to the TUI.

## Architecture

Library crate (`src/lib.rs`) plus thin binary (`src/main.rs`).

| Module | Status | Role |
|---|---|---|
| `audio::extractor` | done | find/download yt-dlp, resolve direct audio URL |
| `audio::resolver` | done | URL cache, dedup, max 2 yt-dlp procs, fallback ladder |
| `audio::js_runtime` | done | optional JS runtime for yt-dlp (QuickJS-NG) |
| `audio::install` | done | SHA-256-verified download, atomic rename |
| `config::paths` | done | per-OS dirs via `directories` |
| `audio::{player,queue,stream}` | stub | rodio thread, queue, HTTP Range reader |
| `api::*` | stub | OAuth PKCE, keyring tokens, YouTube Data API v3, quota |
| `storage::*` | stub | SQLite cache (rusqlite bundled) |
| `sync`, `ui`, `app` | stub | sync service, ratatui TUI, main loop |

Each stub file's doc comment describes its intended design.

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
- Our own process: ~12 MB RSS; release binary 3.7 MB.

## Planned design notes

- Liked music: `videos.list?myRating=like` filtered by `categoryId == "10"`;
  test `playlistItems.list?playlistId=LM|LL`.
- OAuth: "Desktop app" client, PKCE + loopback redirect; device flow as
  SSH fallback. Google project must be "In production" (Testing ⇒ refresh
  tokens expire after 7 days). `client_secret` lives in keyring.
- Quota: 10,000 units/day; use ETags (`If-None-Match`).

## Open TODOs

- `js_fallback = true/false` in `config.toml` (user asked, pending).
- Persist resolved-URL cache in SQLite.
- README with per-OS build setup (system deps).
- Player, TUI, auth, sync, storage implementations.
- Measure real playback RAM once the player exists.

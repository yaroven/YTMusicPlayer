# ytm-player

A lightweight player for YouTube Music — a desktop window styled after
YouTube Music, a terminal UI, and a native iPhone app. macOS, Linux,
Windows, iOS.

- **Browse and play:** home feed, Explore (charts, new releases, moods and
  genres), search with suggestions, album, artist and playlist pages, song
  radio and autoplay, karaoke-style synced lyrics, related music, history.
- **Your library:** liked music, playlists, saved albums and followed artists,
  synced with one **Sign in with Google** — no quota, no setup, nothing
  expiring after a week. Likes, playlist changes and follows go back to
  YouTube.
- **Playback:** gapless or crossfade, loudness normalization like YouTube
  Music, editable queue, sleep timer, offline downloads, Chromecast, media
  keys and the system "Now Playing" widget. A song with a clip can be
  watched in the window and handed back to the audio where the video stopped.
- **English or Ukrainian:** the whole interface, switched in Settings
  (`language` in the config; `auto` follows the system).
- **Light:** the terminal UI takes ~10 MB of memory idle and ~15 MB while
  playing, the window ~50 MB playing and ~30 MB closed to the tray; near-zero
  CPU. A YouTube Music browser tab typically takes several hundred MB.
- **iPhone:** the same core as a SwiftUI app, installed with SideStore —
  [docs/iphone.md](docs/iphone.md).
- Audio comes from [yt-dlp](https://github.com/yt-dlp/yt-dlp), which the app
  downloads and keeps updated by itself.

```
┌ Library ──────────────┐┌ Liked music · 120 ──────────────────────────────┐
│› Liked music 120      ││#      Title                     Artist     Time │
│  Road trip 42         ││    ▶  Never Gonna Give You Up   Rick Astley 3:33│
│  Focus 87             ││    2  ♥ Despacito               Luis Fonsi  4:41│
└───────────────────────┘└─────────────────────────────────────────────────┘
┌ ▶ Never Gonna Give You Up — Rick Astley ──── shuffle repeat vol  80% ┐
│0:22 / 3:33 ━━━━━━━━━──────────────────────────────────────────────────── │
└──────────────────────────────────────────────────────────────────── 1/120 ┘
```

## Quick start

1. **Install** the file for your system from the
   [latest release](https://github.com/yaroven/YTMusicPlayer/releases/latest)
   (table below).
2. **Open ytm-player.** Search and playback work right away. The **Google
   account** dialog offers **Sign in with Google**, which brings in your
   library and lets age-restricted songs play —
   [docs/google-account.md](docs/google-account.md).

| System | File | Install |
|---|---|---|
| Debian, Ubuntu, Mint | `…_amd64.deb` | `sudo apt install ./ytm-player_*_amd64.deb` |
| Fedora | `…x86_64.rpm` | `sudo dnf install ./ytm-player-*.x86_64.rpm` |
| Arch, Manjaro | `…pkg.tar.zst` | `sudo pacman -U ./ytm-player-*.pkg.tar.zst` |
| Windows 10/11 | `…-setup-x64.exe` | run it (no admin rights) |
| macOS 11+ | `…-macos.pkg` | open it |
| iPhone (iOS 17+) | SideStore source | [docs/iphone.md](docs/iphone.md) |

The installers aren't signed with a paid certificate, so macOS and Windows
each ask once — that, the install scripts and building from source are in
[docs/install.md](docs/install.md).

## Documentation

| | |
|---|---|
| [install.md](docs/install.md) | installers, install scripts, building, updates, uninstall |
| [usage.md](docs/usage.md) | the window, the terminal UI and its keys, `ytm` commands |
| [google-account.md](docs/google-account.md) | signing in, library sync, the optional Data API |
| [configuration.md](docs/configuration.md) | `config.toml`, Chromecast, Last.fm, Discord, offline |
| [iphone.md](docs/iphone.md) | the iPhone app: install, sign in, limits |
| [troubleshooting.md](docs/troubleshooting.md) | age-restricted songs, sound, sign-in, playback |
| [internals.md](docs/internals.md) | how the player works inside, flow by flow |

## Notes and limits

- "Liked music" is YouTube Music's own liked list. Without the Google
  sign-in, the optional Data API path shows your liked YouTube videos instead,
  filtered to the Music category unless `liked_music_only = false`; the sync
  message says which.
- Audio is AAC ~128 kbps. Opus would need a C library on every platform for no
  audible gain, so it isn't used.
- yt-dlp runs as a short-lived child process, one at a time (~90 MB for a
  couple of seconds while it fetches a track link).
- On Windows, media keys work in the window, not in the terminal UI.
- Chromecast is tested against a simulated receiver only so far.
- Linux packages are x86_64 only; other architectures build from source with
  `install.sh`.

## Development

```bash
cargo build --release     # binary: target/release/ytm
cargo test                # unit, window and terminal-UI tests
cargo test -- --ignored   # tests that hit the network
```

Architecture, decisions and the testing setup are in
[CLAUDE.md](CLAUDE.md); domain terms in [CONTEXT.md](CONTEXT.md). The iPhone
app lives in [`ios/`](ios) and is built by CI.
